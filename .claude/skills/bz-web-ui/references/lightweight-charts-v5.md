# lightweight-charts v5 (5.3, ESM only)

Open this before writing chart code. API names below were checked against the official v4-to-v5 migration page and the v5 API reference (TradingView docs). Sub-field lists (marker shapes, chart options) come from the reference; check the installed version's types when something does not compile.

## What changed from v4 (do not write v4 code)

- Series are created with `chart.addSeries(Definition, options, paneIndex?)`. The old `addLineSeries`, `addCandlestickSeries`, `addHistogramSeries`, `addAreaSeries`, `addBarSeries`, `addBaselineSeries` are gone.
- Definitions are ESM exports: `CandlestickSeries`, `LineSeries`, `HistogramSeries`, `AreaSeries`, `BarSeries`, `BaselineSeries`. Import them next to `createChart`. The package is ESM only, so no `require`, and under Vitest the module resolves normally.
- `series.setMarkers()` is gone. Use `createSeriesMarkers(series, markers)` which returns a plugin object with `setMarkers(markers)` (empty array clears) and `markers()`.
- The `watermark` chart option is gone; use `createTextWatermark` / `createImageWatermark` if one is ever needed (the disclaimer is NOT a watermark: it lives in the shell).
- Multiple panes are built in: `addSeries(HistogramSeries, opts, 1)` places the series in pane 1 (created on demand). `chart.panes()`, `chart.addPane()`, `chart.removePane(i)`, `series.moveToPane(i)`, `series.getPane()`.

## Wrapper component pattern

One imperative component owns the chart. React renders a container div; everything else happens in effects.

```tsx
import { createChart, CandlestickSeries, HistogramSeries, createSeriesMarkers, ColorType,
         type IChartApi, type ISeriesApi, type ISeriesMarkersPluginApi, type IPriceLine, type Time } from "lightweight-charts";

export function PriceChart({ candles, markers, levels }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const api = useRef<{ chart: IChartApi; candle: ISeriesApi<"Candlestick">; volume: ISeriesApi<"Histogram">;
                       markers: ISeriesMarkersPluginApi<Time>; lines: IPriceLine[] } | null>(null);

  useEffect(() => {                                   // create once
    const chart = createChart(host.current!, {
      autoSize: true,                                 // ResizeObserver-driven; no manual resize()
      layout: { background: { type: ColorType.Solid, color: "transparent" }, textColor: "#d1d5db" },
      localization: { timeFormatter: (t: Time) => formatIst(t as number) },
    });
    const candle = chart.addSeries(CandlestickSeries, { upColor: "#16a34a", downColor: "#dc2626", borderVisible: false });
    const volume = chart.addSeries(HistogramSeries, { priceFormat: { type: "volume" } }, 1);   // pane 1
    api.current = { chart, candle, volume, markers: createSeriesMarkers(candle, []), lines: [] };
    return () => { chart.remove(); api.current = null; };   // irreversible; never reuse
  }, []);

  useEffect(() => {                                   // data: replace
    api.current?.candle.setData(candles);             // sorted ascending by time, unique times
  }, [candles]);
  // markers and levels: separate effects, see below
  return <div ref={host} className="chart-host" role="img" aria-label={summary(candles)} />;
}
```

Rules:

- `autoSize: true` makes the chart follow its container; `chart.resize(w, h)` is then ignored. The container needs a real height from CSS (flex or fixed), otherwise the chart is 0 px tall.
- React 19 StrictMode mounts effects twice in dev: the cleanup `chart.remove()` makes that safe. Never call methods on a removed chart.
- Live updates: `series.update(bar)` accepts only a time >= the last bar (same time replaces it). For an earlier point pass `historicalUpdate = true` (slower). Initial or reloaded history uses `setData`.
- Do not recreate the chart when the symbol or timeframe changes; call `setData` and `timeScale().fitContent()`.

## Time, IST and data shape

- Intraday bars: `time` is UTC epoch seconds (`UTCTimestamp`). Keep real UTC values, do not shift them by +5:30 to fake IST; instead set `localization.timeFormatter` (crosshair) and `timeScale.tickMarkFormatter` to format with `Intl.DateTimeFormat` and `timeZone: "Asia/Kolkata"`.
- Daily and longer bars: use the string `"YYYY-MM-DD"` (the IST trading date from the backend) as a business day, which avoids time-zone drift on EOD charts.
- Prices are numbers inside the chart only because the library requires it: convert from `Money` strings at the chart boundary (`Number(price)`), display-only. Tooltips and labels outside the canvas use `formatInr` on the original strings. Set `priceFormat` for tick precision from the instrument tick size supplied by the backend, never hardcoded.
- Candle rows, volume rows and indicator rows come from the chart endpoints in `bz-web-api` `references/api-contract.md` (typed from `schema.d.ts`); indicators are computed in Rust (`bz-quant`), never recomputed in the browser.
- Live bars: ticks arrive over the WebSocket into the tick store; coalesce to one `series.update` per animation frame (or about 250 ms) instead of one per tick, and push only the latest bar.

## Markers (strategy entries and exits)

```ts
api.current!.markers.setMarkers(
  signals.map((s) => ({
    time: s.time as Time,
    position: s.side === "BUY" ? "belowBar" : "aboveBar",
    shape: s.side === "BUY" ? "arrowUp" : "arrowDown",
    color: s.side === "BUY" ? "#16a34a" : "#dc2626",
    text: s.label,                                   // short: "Buy ORB", "Exit SL"
  })),
);
```

Markers must be sorted by time. Keep one marker plugin per series (created once, `setMarkers` on change). Entries and exits use different shapes (arrow vs circle) so they remain distinguishable without colour.

## Price lines (stop, targets, entry zone, S/R, OI walls)

`series.createPriceLine({ price, color, lineWidth, lineStyle, axisLabelVisible, title })` returns an `IPriceLine`; remove with `series.removePriceLine(line)`. Keep the created lines in a ref array and remove all of them before drawing new ones when the recommendation or levels change, otherwise they accumulate. Convention: stop = red dashed titled "Stop", targets = green titled "T1/T2", entry zone = two thin lines, S/R = grey solid, OI walls = purple dotted titled "Call OI wall 24,500". Titles should be plain English. A 5.3 option on price lines, `hitTestTolerance`, controls hover sensitivity (reported in the 5.3.0 release notes; confirm in the type definitions).

## Panes for indicators

Overlays (EMA, Supertrend, VWAP, Bollinger) are `LineSeries` on pane 0. Oscillators (RSI, MACD, ADX) go in pane 1 or 2 via the third `addSeries` argument; volume in its own pane or as an overlay histogram with a separate price scale. Adjust pane heights with the pane API (`chart.panes()[i].setHeight(px)`; verify method availability on the installed version). Toggling an indicator off removes its series (`chart.removeSeries(series)`) rather than hiding by transparency.

## Other chart screens

- Equity curve and drawdown (backtest lab): `AreaSeries` or `LineSeries` + `BaselineSeries`; monthly-returns heatmap is plain HTML table, not a chart.
- Payoff diagram (option strategy builder): x-axis is a price, not time. lightweight-charts is time-based; use a plain SVG (or a small canvas component) for payoff and OI bars rather than bending the library. Keep curve points computed in the `options` crate and returned by the API.
- Sector heatmap: HTML/CSS grid.

## Testing

jsdom has no canvas. In Vitest, `vi.mock("lightweight-charts", ...)` with fakes for `createChart` (returning `addSeries`, `remove`, `timeScale`, `applyOptions`), `createSeriesMarkers` and the series (`setData`, `update`, `createPriceLine`, `removePriceLine`). Assert: chart created once, `remove` called once on unmount, `setData` receives sorted data, old price lines removed before new ones, markers set via `createSeriesMarkers` and not `setMarkers` on the series. Real rendering is checked only in the Playwright smoke screenshot against the MSW-mocked app (`bz-testing` `references/ui-mocked-api.md`). The Playwright CSP spec decides whether the chart needs any CSP change; do not widen the policy in code (the `style-src-attr` question is owned by `bz-security`).
