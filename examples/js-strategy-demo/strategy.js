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
 *
 * 入口函数要求:
 * - 必须定义全局函数 `onCandle(input)`
 * - input 是当前最新闭合的 K 线 JSON 序列化字符串
 * - 必须返回有效的 JSON 字符串，代表 `Signal`，如果你不想发任何信号，可以返回 "null"
 */

function onCandle(input) {
    // 1. 解析当前 K 线
    var candle = JSON.parse(input);

    // 如果不是最终确认的 K 线，可以直接跳过计算
    if (!candle.is_final) {
        return "null";
    }

    // 2. 调用系统能力：打印日志
    host.log(3, "Processing candle closed at: " + candle.close);

    // 3. 调用系统能力：取历史数据用来算移动平均
    // 注意：获取历史数据是比较耗时的桥接操作，在实际高频中可能会用全局变量自己做增量缓存计算
    var historyJson = host.fetchHistory("AAPL", "1m", 10);
    var history = JSON.parse(historyJson);

    if (history.length < 10) {
        // 数据不够，不发信号
        host.log(2, "Not enough history data, skipping logic");
        return "null";
    }

    // 简单计算一下历史前10根K线的平均收盘价 (SMA10)
    var sum = 0.0;
    for (var i = 0; i < history.length; i++) {
        sum += history[i].close;
    }
    var sma10 = sum / history.length;

    // 4. 业务逻辑与发信判断
    // 逻辑：如果本根 K 线强势站上过去10分钟均线，并且成交量放大，产生买入信号
    if (candle.close > sma10 && candle.volume > 500) {

        // 5. 调用系统能力：取得确切的策略计算时钟
        var logicTime = host.now();
        var tsString = new Date(logicTime).toISOString();

        host.log(3, "Triggering LongEntry! Close: " + candle.close + " > SMA10: " + sma10);

        // 返回包含交易信号的 JSON 字符串
        return JSON.stringify({
            id: "sig_js_" + logicTime,         // 确保发出去的 ID 基本不重样
            symbol: "AAPL",
            timestamp: tsString,               // 使用标准 UTC 时间字符串
            kind: "LongEntry",                 // 信号类型
            strategy_id: "js-ema-breakout",
            metadata: {
                "triggered_price": String(candle.close),
                "sma_value": String(sma10)
            }
        });
    }

    // 不满足条件，什么都不做
    return "null";
}
