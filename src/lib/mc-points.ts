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

const EARTH_RADIUS_M = 6_371_000

const CONTAINMENT_GREEN = 0.68
const CONTAINMENT_YELLOW = 0.95

export function ensembleContainments(
  points: MonteCarloPoint[],
): number[] | null {
  const n = points.length
  if (n < 3) return null

  let latMean = 0
  let lonMean = 0
  for (const p of points) {
    latMean += p.landing_lat
    lonMean += p.landing_lon
  }
  latMean /= n
  lonMean /= n

  const degToRad = Math.PI / 180
  const cosLat = Math.cos(latMean * degToRad)
  const xs: number[] = new Array(n)
  const ys: number[] = new Array(n)
  for (let i = 0; i < n; i++) {
    xs[i] =
      EARTH_RADIUS_M * (points[i].landing_lon - lonMean) * degToRad * cosLat
    ys[i] = EARTH_RADIUS_M * (points[i].landing_lat - latMean) * degToRad
  }

  let sxx = 0
  let syy = 0
  let sxy = 0
  for (let i = 0; i < n; i++) {
    sxx += xs[i] * xs[i]
    syy += ys[i] * ys[i]
    sxy += xs[i] * ys[i]
  }
  sxx /= n - 1
  syy /= n - 1
  sxy /= n - 1

  if (Math.sqrt(Math.max(sxx, syy)) < 0.01) return null

  const det = sxx * syy - sxy * sxy
  if (!(det > 0)) return null

  const invSxx = syy / det
  const invSyy = sxx / det
  const invSxy = -sxy / det

  const out: number[] = new Array(n)
  for (let i = 0; i < n; i++) {
    const dx = xs[i]
    const dy = ys[i]
    const m2 = invSxx * dx * dx + 2 * invSxy * dx * dy + invSyy * dy * dy
    out[i] = Math.min(1, Math.max(0, 1 - Math.exp(-Math.max(m2, 0) / 2)))
  }
  return out
}

export function containmentToDeckColor(u: number): DeckColor {
  if (u <= CONTAINMENT_GREEN) return [34, 197, 94, 200]
  if (u <= CONTAINMENT_YELLOW) return [234, 179, 8, 200]
  return [239, 68, 68, 200]
}

export function containmentToDotClass(u: number): string {
  if (u <= CONTAINMENT_GREEN) return "bg-[rgb(34,197,94)]"
  if (u <= CONTAINMENT_YELLOW) return "bg-[rgb(234,179,8)]"
  return "bg-[rgb(239,68,68)]"
}

export function pointFillColor(
  point: MonteCarloPoint,
  useSigma: boolean,
  containment?: number | null,
): DeckColor {
  if (useSigma && point.deviation_sigma != null) {
    return sigmaToDeckColor(point.deviation_sigma)
  }
  if (containment != null) {
    return containmentToDeckColor(containment)
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
