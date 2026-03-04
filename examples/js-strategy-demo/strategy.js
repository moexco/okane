/**
 * 标准 JS 策略示例
 *
 * 这是一份可以在 okane_engine (JsEngine) 中直接加载执行的策略源码模板。
 * 策略名称: EMA Breakout Demo
 *
 * 可用的宿主接口 (由 JsEngine 提供):
 * - host.now() -> number (当前 K 线逻辑时间的毫秒时间戳)
 * - host.log(level: number, msg: string) -> void (1=ERROR, 2=WARN, 其他=INFO)
 * - host.fetchHistory(symbol: string, tf: string, limit: number) -> string (只读的 JSON 历史 K 线数组)
 * - host.buy(symbol: string, price: number|null, volume: number) -> string (下买单, 返回 order_id JSON)
 * - host.sell(symbol: string, price: number|null, volume: number) -> string (下卖单, 返回 order_id JSON)
 * - host.getAccount() -> string (查询账户快照 JSON)
 * - host.getOrder(orderId: string) -> string (查询订单详情 JSON)
 * - host.cancelOrder(orderId: string) -> string (撤单, "ok" 或 error JSON)
 * - host.notify(subject: string, content: string) -> string (推送通知)
 *
 * 入口函数要求:
 * - 必须定义全局函数 `onCandle(input)`
 * - input 是当前最新闭合的 K 线 JSON 序列化字符串
 * - onCandle 为 void 函数，策略通过 host.* API 直接执行动作
 */

function onCandle(input) {
    // 1. 解析当前 K 线
    var candle = JSON.parse(input);

    // 如果不是最终确认的 K 线，可以直接跳过计算
    if (!candle.is_final) {
        return;
    }

    // 2. 调用系统能力：打印日志
    host.log(3, "Processing candle closed at: " + candle.close);

    // 3. 调用系统能力：取历史数据用来算移动平均
    var historyJson = host.fetchHistory("AAPL", "1m", 10);
    var history = JSON.parse(historyJson);

    if (history.length < 10) {
        host.log(2, "Not enough history data, skipping logic");
        return;
    }

    // 简单计算一下历史前10根K线的平均收盘价 (SMA10)
    var sum = 0.0;
    for (var i = 0; i < history.length; i++) {
        sum += history[i].close;
    }
    var sma10 = sum / history.length;

    // 4. 业务逻辑：如果本根 K 线强势站上过去10分钟均线，并且成交量放大，买入
    if (candle.close > sma10 && candle.volume > 500) {
        var logicTime = host.now();

        host.log(3, "Triggering BUY! Close: " + candle.close + " > SMA10: " + sma10);

        // 通过 host.buy 直接下单 (市价单, 100股)
        var result = host.buy("AAPL", null, 100);
        host.log(0, "Buy order result: " + result);

        // 通知
        host.notify("EMA Breakout", "AAPL 突破 SMA10, close=" + candle.close);
    }
}
