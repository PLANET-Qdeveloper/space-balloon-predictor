import { useMemo, useRef } from "react"
import {
  ResponsiveContainer,
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ReferenceLine,
  ReferenceDot,
} from "recharts"
import type { TrajectoryPoint } from "@/types"

interface AltitudeTimeChartProps {
  ascent: TrajectoryPoint[]
  descent: TrajectoryPoint[]
  meanAscent?: TrajectoryPoint[]
  meanDescent?: TrajectoryPoint[]
  launchTimeUtc?: string | null
  onPointHover?: (point: (TrajectoryPoint & { leg: "ascent" | "descent" }) | null) => void
}

const TROPOPAUSE_M = 11_000

function formatJst(launchTimeUtc: string | null | undefined, elapsedMin: number): string | null {
  if (!launchTimeUtc) return null
  const base = Date.parse(launchTimeUtc)
  if (isNaN(base)) return null
  const at = new Date(base + elapsedMin * 60_000)
  const hhmm = at.toLocaleString("ja-JP", {
    timeZone: "Asia/Tokyo",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  })
  return `${hhmm} JST`
}

function toPoints(path: TrajectoryPoint[]): { t: number; alt: number; lat: number; lon: number }[] {
  return path
    .filter((p) => p.time !== undefined && p.alt !== undefined)
    .map((p) => ({ t: (p.time ?? 0) / 60, alt: p.alt, lat: p.lat, lon: p.lon }))
}

type ChartPoint = { t: number; alt: number; lat: number; lon: number; leg: "ascent" | "descent"; meanAlt?: number | null }

function tagLeg(points: { t: number; alt: number; lat: number; lon: number }[], leg: "ascent" | "descent"): ChartPoint[] {
  return points.map((p) => ({ ...p, leg }))
}

function formatTickMin(v: number): string {
  return `${v}分`
}

export function AltitudeTimeChart({
  ascent,
  descent,
  meanAscent,
  meanDescent,
  launchTimeUtc = null,
  onPointHover,
}: AltitudeTimeChartProps) {
  const lastEmittedRef = useRef<
    { lat: number; lon: number; alt: number; time: number; leg: "ascent" | "descent" } | null
  >(null)
  const ascentData = useMemo(() => toPoints(ascent), [ascent])
  const descentData = useMemo(() => toPoints(descent), [descent])
  const meanAscentData = useMemo(() => (meanAscent ? toPoints(meanAscent) : []), [meanAscent])
  const meanDescentData = useMemo(() => (meanDescent ? toPoints(meanDescent) : []), [meanDescent])

  const data = useMemo<ChartPoint[]>(() => {
    const meanPts = [...meanAscentData, ...meanDescentData]
    const nearestMeanAlt = (t: number): number | null => {
      if (meanPts.length === 0) return null
      let best = meanPts[0].alt
      let bestDist = Infinity
      for (const p of meanPts) {
        const d = Math.abs(p.t - t)
        if (d < bestDist) {
          bestDist = d
          best = p.alt
        }
      }
      return best
    }
    return [
      ...tagLeg(ascentData, "ascent").map((p) => ({ ...p, meanAlt: nearestMeanAlt(p.t) })),
      ...tagLeg(descentData.slice(1), "descent").map((p) => ({ ...p, meanAlt: nearestMeanAlt(p.t) })),
    ]
  }, [ascentData, descentData, meanAscentData, meanDescentData])

  const hasTime = ascentData.length > 0 || descentData.length > 0

  const burst = useMemo(() => {
    if (ascentData.length === 0) return null
    return ascentData[ascentData.length - 1]
  }, [ascentData])

  const maxAlt = useMemo(() => {
    const all = [...ascentData, ...descentData, ...meanAscentData, ...meanDescentData]
    if (all.length === 0) return 0
    return all.reduce((m, p) => (p.alt > m ? p.alt : m), 0)
  }, [ascentData, descentData, meanAscentData, meanDescentData])

  const yMax = Math.ceil(maxAlt / 5000) * 5000
  const yTicks = useMemo(() => {
    const ticks: number[] = []
    for (let v = 0; v <= yMax; v += 5000) ticks.push(v)
    return ticks
  }, [yMax])

  const maxT = useMemo(() => {
    const all = [...ascentData, ...descentData]
    if (all.length === 0) return 0
    return all.reduce((m, p) => (p.t > m ? p.t : m), 0)
  }, [ascentData, descentData])

  const xTicks = useMemo(() => {
    const ticks: number[] = []
    for (let v = 0; v <= maxT; v += 30) ticks.push(v)
    return ticks
  }, [maxT])

  const hasMean = meanAscentData.length > 0 || meanDescentData.length > 0

  if (!hasTime) {
    return (
      <p className="text-xs text-muted-foreground py-6 text-center">
        高度プロファイルのデータがありません
      </p>
    )
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <p className="text-xs text-muted-foreground">高度プロファイル</p>
        <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1">
            <span className="w-2 h-0.5 bg-[rgb(239,68,68)]" />
            上昇
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-0.5 bg-[rgb(59,130,246)]" />
            下降
          </span>
          {hasMean && (
            <span className="flex items-center gap-1">
              <span className="w-2 h-0.5 border-t border-dashed border-muted-foreground" />
              平均
            </span>
          )}
        </div>
      </div>
      <ResponsiveContainer width="100%" height={180} className="outline-none border-none [&_svg]:outline-none [&_svg]:border-none">
        <LineChart
          data={data}
          margin={{ top: 0, right: 0, bottom: 0, left: 0 }}
          className="outline-none border-none"
        >
          <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
          <XAxis
            dataKey="t"
            type="number"
            domain={[0, maxT]}
            ticks={xTicks}
            tickFormatter={formatTickMin}
            tick={{ fontSize: 11 }}
            tickLine={false}
            axisLine={{ stroke: "var(--border)" }}
            padding="no-gap"
          />
          <YAxis
            type="number"
            domain={[0, yMax]}
            ticks={yTicks}
            tickFormatter={(v: number) => `${v / 1000}km`}
            tick={{ fontSize: 11 }}
            tickLine={false}
            axisLine={false}
            width={40}
            padding="no-gap"
          />
          <Tooltip
            cursor={false}
            content={(props) => {
              const active = props.active
              const payload = props.payload as unknown as
                | { payload?: ChartPoint; value?: number }[]
                | undefined
              const datum = active && payload?.length ? payload[0].payload : undefined
              if (onPointHover) {
                const hasPos =
                  datum &&
                  Number.isFinite(datum.lat) &&
                  Number.isFinite(datum.lon) &&
                  Number.isFinite(datum.alt)
                const leg: "ascent" | "descent" = datum?.leg === "descent" ? "descent" : "ascent"
                const emitted = hasPos
                  ? { lat: datum!.lat, lon: datum!.lon, alt: datum!.alt, time: datum!.t * 60, leg }
                  : null
                const prev = lastEmittedRef.current
                const unchanged =
                  (emitted === null && prev === null) ||
                  (emitted !== null &&
                    prev !== null &&
                    prev.lat === emitted.lat &&
                    prev.lon === emitted.lon &&
                    prev.alt === emitted.alt &&
                    prev.time === emitted.time &&
                    prev.leg === emitted.leg)
                if (!unchanged) {
                  lastEmittedRef.current = emitted
                  onPointHover(emitted)
                }
              }
              const label = active && props.label !== undefined ? props.label : null
              const value = active && payload?.length ? payload[0].value : undefined
              const jst =
                label !== null && Number.isFinite(Number(label))
                  ? formatJst(launchTimeUtc, Number(label))
                  : null
              return (
                <div
                  style={{
                    fontSize: 12,
                    borderRadius: 6,
                    background: "var(--background)",
                    border: "1px solid var(--border)",
                    color: "var(--foreground)",
                    boxShadow: "0 2px 8px rgb(0 0 0 / 15%)",
                    padding: "4px 8px",
                  }}
                >
                  {label !== null && (
                    <p className="text-muted-foreground">{`${Number(label).toFixed(1)}分`}</p>
                  )}
                  {jst !== null && (
                    <p className="text-muted-foreground">{jst}</p>
                  )}
                  {value !== undefined && (
                    <p>{`${Math.round(Number(value)).toLocaleString()} m`}</p>
                  )}
                </div>
              )
            }}
          />
          <ReferenceLine
            y={TROPOPAUSE_M}
            stroke="var(--muted-foreground)"
            strokeDasharray="4 4"
            strokeOpacity={0.5}
            label={{ value: "対流圏界 11km", position: "insideTopRight", fontSize: 10, fill: "var(--muted-foreground)" }}
          />
          {hasMean && (
            <Line
              dataKey={(d: ChartPoint) => (d.meanAlt != null ? d.meanAlt : null)}
              type="monotone"
              stroke="var(--muted-foreground)"
              strokeWidth={1}
              strokeDasharray="4 4"
              dot={false}
              activeDot={false}
              tooltipType="none"
              isAnimationActive={false}
            />
          )}
          <Line
            dataKey={(d: ChartPoint) => (d.leg === "ascent" ? d.alt : null)}
            type="monotone"
            stroke="rgb(239,68,68)"
            strokeWidth={1.5}
            dot={false}
            isAnimationActive={false}
          />
          <Line
            dataKey={(d: ChartPoint) => (d.leg === "descent" ? d.alt : null)}
            type="monotone"
            stroke="rgb(59,130,246)"
            strokeWidth={1.5}
            dot={false}
            isAnimationActive={false}
          />
          {burst && (
            <ReferenceDot
              x={burst.t}
              y={burst.alt}
              r={3}
              fill="rgb(234,179,8)"
              stroke="var(--background)"
              strokeWidth={1}
            />
          )}
        </LineChart>
      </ResponsiveContainer>
    </div>
  )
}
