use okane_core::trade::entity::{AccountId, AccountSnapshot, Position};
use okane_core::trade::port::TradeError;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// # Summary
/// 系统核心内部账户的并发安全封装对象。
/// 通过 RwLock 保护内部状态以防御高并发条件下的竞态数据错乱。
pub struct AccountState {
    pub account_id: AccountId,
    /// 可用现金 (可用于新开单的额度)
    pub available_balance: Decimal,
    /// 冻结资金 (已被在途开单挂起，尚未成交扣款的部分)
    pub frozen_balance: Decimal,
    /// 单个标的的持仓记录映射
    pub positions: HashMap<String, Position>,
}

impl AccountState {
    pub fn new(account_id: AccountId, initial_balance: Decimal) -> Self {
        Self {
            account_id,
            available_balance: initial_balance,
            frozen_balance: Decimal::ZERO,
            positions: HashMap::new(),
        }
    }

    /// # Logic
    /// 开仓挂单时，冻结相应的准备金。
    pub fn freeze_funds(&mut self, amount: Decimal) -> Result<(), TradeError> {
        if self.available_balance < amount {
            return Err(TradeError::InsufficientFunds {
                required: amount,
                actual: self.available_balance,
            });
        }
        self.available_balance -= amount;
        self.frozen_balance += amount;
        Ok(())
    }

    /// # Logic
    /// 撤单时解冻准备金，归还到可用余额。
    pub fn unfreeze_funds(&mut self, amount: Decimal) {
        // 如果系统正常运行，amount 不应超过 frozen_balance，但此处做个防御
        let actual_unfreeze = if amount > self.frozen_balance {
            tracing::warn!(
                "账户 {} 解冻异常: 试图解冻 {} 但仅剩 {}",
                self.account_id.0,
                amount,
                self.frozen_balance
            );
            self.frozen_balance
        } else {
            amount
        };
        self.frozen_balance -= actual_unfreeze;
        self.available_balance += actual_unfreeze;
    }

    /// # Logic
    /// 实际发生成交时扣款（如买入扣款），从冻结资金中扣除。如果不够，尝试扣可用余额。
    pub fn deduct_funds(&mut self, target_amount: Decimal) {
        if self.frozen_balance >= target_amount {
            self.frozen_balance -= target_amount;
        } else {
            let remain = target_amount - self.frozen_balance;
            self.frozen_balance = Decimal::ZERO;
            // 极限情况下（如滑点极大导致超过开单前预期冻结值），扣减可用资金
            self.available_balance -= remain;
        }
    }

    /// # Logic
    /// 到账/增加现金（如卖出所得、分红）。
    pub fn add_funds(&mut self, amount: Decimal) {
        self.available_balance += amount;
    }

    /// # Logic
    /// 调整目标证券的持仓数量。对于平仓操作可能直接抹平持仓。
    pub fn update_position(&mut self, symbol: &str, delta_volume: Decimal, trade_price: Decimal) {
        if delta_volume.is_zero() {
            return;
        }
        
        let position = self.positions.entry(symbol.to_string()).or_insert_with(|| Position {
            account_id: self.account_id.clone(),
            symbol: symbol.to_string(),
            volume: Decimal::ZERO,
            average_price: Decimal::ZERO,
        });

        // 多头买入或空头卖出（开仓动作，通常会增加头寸绝对值，更新平均价）
        if (position.volume.is_sign_positive() && delta_volume.is_sign_positive())
            || (position.volume.is_sign_negative() && delta_volume.is_sign_negative())
            || position.volume.is_zero()
        {
            let old_cost = position.volume.abs() * position.average_price;
            let added_cost = delta_volume.abs() * trade_price;
            position.volume += delta_volume;
            if !position.volume.is_zero() {
                position.average_price = (old_cost + added_cost) / position.volume.abs();
            }
        } else {
            // 平仓动作，头寸减少，平均成本不变，仅扣减数量
            position.volume += delta_volume;
            // 如果头寸被平光，甚至是反向开新仓，重置价格（简化处理，真实往往拆为平仓和开仓两笔流水）
            if position.volume.is_zero() {
                position.average_price = Decimal::ZERO;
            } else if (position.volume.is_sign_positive() && delta_volume.is_sign_negative())
                || (position.volume.is_sign_negative() && delta_volume.is_sign_positive())
            {
                // 如果刚好反手了
                position.average_price = trade_price;
            }
        }
    }

    /// # Logic
    /// 获取对外透明的只读快照数据。
    pub fn to_snapshot(&self) -> AccountSnapshot {
        // 未实现总权益动态浮盈计算，先简单求和当前现金作为 placeholder
        let total_equity = self.available_balance + self.frozen_balance;

        AccountSnapshot {
            account_id: self.account_id.clone(),
            available_balance: self.available_balance,
            frozen_balance: self.frozen_balance,
            total_equity,
            positions: self.positions.values().cloned().collect(),
        }
    }
}

/// # Summary
/// OMS 本地的系统账户管理器。负责所有活跃物理/逻辑账号的并发调度保护。
pub struct AccountManager {
    /// 全局活动的账户大盘。使用 DashMap 实现细粒度的分段锁，
    /// 其内部再使用 RwLock 来做单户读写一致性锁。
    accounts: dashmap::DashMap<AccountId, Arc<RwLock<AccountState>>>,
}

impl Default for AccountManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountManager {
    pub fn new() -> Self {
        Self {
            accounts: dashmap::DashMap::new(),
        }
    }

    /// 加载或新建某个模拟/纸面系统账户并注入初始资金。
    pub fn ensure_account_exists(&self, id: AccountId, initial_balance: Decimal) {
        if !self.accounts.contains_key(&id) {
            let state = AccountState::new(id.clone(), initial_balance);
            self.accounts.insert(id, Arc::new(RwLock::new(state)));
        }
    }

    /// # Logic
    /// 获取某根账户的强共享读写互斥锁代理。
    /// 可以使用 tokio::sync::RwLock 的异步 wait，绝不形成同步阻塞点。
    pub fn get_account(&self, id: &AccountId) -> Result<Arc<RwLock<AccountState>>, TradeError> {
        self.accounts
            .get(id)
            .map(|kv| kv.value().clone())
            .ok_or_else(|| TradeError::AccountNotFound(id.0.clone()))
    }

    /// 获取对外账户快照。
    pub async fn snapshot(&self, id: &AccountId) -> Result<AccountSnapshot, TradeError> {
        let acct_lock = self.get_account(id)?;
        let state = acct_lock.read().await;
        Ok(state.to_snapshot())
    }
}
