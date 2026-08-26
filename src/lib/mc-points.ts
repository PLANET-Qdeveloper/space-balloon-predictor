import type { MonteCarloPoint } from "@/types"

export type DeckColor = [number, number, number, number]

export const NEUTRAL_POINT_COLOR: DeckColor = [59, 130, 246, 220]

export function hasSigma(points: MonteCarloPoint[]): boolean {
  return (
    points.length > 0 &&
    points.every(
      (p) => p.deviation_sigma !== null && p.deviation_sigma !== undefined,
    )
  )
}

export function sigmaToDeckColor(sigma: number): DeckColor {
  const abs = Math.abs(sigma)
  if (abs <= 1) return [34, 197, 94, 200]
  if (abs <= 2) return [234, 179, 8, 200]
  return [239, 68, 68, 200]
}

export function sigmaToDotClass(sigma: number): string {
  const abs = Math.abs(sigma)
  if (abs <= 1) return "bg-[rgb(34,197,94)]"
  if (abs <= 2) return "bg-[rgb(234,179,8)]"
  return "bg-[rgb(239,68,68)]"
}

export function pointFillColor(
  point: MonteCarloPoint,
  useSigma: boolean,
): DeckColor {
  if (useSigma && point.deviation_sigma != null) {
    return sigmaToDeckColor(point.deviation_sigma)
  }
  return NEUTRAL_POINT_COLOR
}

export function findDefaultSelectedIndex(
  points: MonteCarloPoint[],
): number | null {
  if (!points || points.length === 0) return null
  if (!hasSigma(points)) return 0
  let bestIdx = 0
  let minDiff = Math.abs(points[0]?.deviation_sigma ?? 0)
  for (let i = 1; i < points.length; i++) {
    const diff = Math.abs(points[i]?.deviation_sigma ?? 0)
    if (diff < minDiff) {
      minDiff = diff
      bestIdx = i
    }
  }
  return bestIdx
}
