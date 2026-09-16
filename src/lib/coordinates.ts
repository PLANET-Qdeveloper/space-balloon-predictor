export type CoordinateAxis = "latitude" | "longitude"

export interface SexagesimalParts {
  degrees: string
  minutes: string
  seconds: string
}

type Direction = "N" | "S" | "E" | "W"

const DMS_COORDINATE_PATTERN =
  /^([+-]?\d+)\s*(?:°|º|度|:)\s*(\d+(?:\.\d*)?)\s*(?:['′’]|分|:)\s*(\d+(?:\.\d*)?)\s*(?:["″”]|秒)?$/
const DECIMAL_COORDINATE_PATTERN =
  /^([+-]?(?:\d+(?:\.\d*)?|\.\d+))\s*(?:°|º|度)?$/

function coordinateLimit(axis: CoordinateAxis): number {
  return axis === "latitude" ? 90 : 180
}

function isValidCoordinate(coordinate: number, axis: CoordinateAxis): boolean {
  return Number.isFinite(coordinate) && Math.abs(coordinate) <= coordinateLimit(axis)
}

function isAllowedDirection(direction: Direction, axis: CoordinateAxis): boolean {
  return axis === "latitude"
    ? direction === "N" || direction === "S"
    : direction === "E" || direction === "W"
}

function directionSign(direction: Direction): number {
  return direction === "S" || direction === "W" ? -1 : 1
}

function directionFromJapanesePrefix(prefix: string): Direction {
  switch (prefix) {
    case "北緯":
      return "N"
    case "南緯":
      return "S"
    case "東経":
      return "E"
    case "西経":
      return "W"
    default:
      throw new Error(`Unknown coordinate direction prefix: ${prefix}`)
  }
}

/**
 * Parses decimal degrees and degree-minute-second coordinates.
 *
 * Examples: `33.12345`, `33°07′30″`, `33:07:30N`, `北緯33度07分30秒`
 */
export function parseCoordinate(
  value: string,
  axis: CoordinateAxis,
): number | undefined {
  const normalized = value.trim().replace(/−/g, "-")
  if (!normalized) return undefined

  let body = normalized
  let direction: Direction | undefined

  const suffixMatch = body.match(/([NSEW])$/i)
  if (suffixMatch) {
    direction = suffixMatch[1].toUpperCase() as Direction
    body = body.slice(0, -suffixMatch[1].length).trim()
  }

  const prefixMatch = body.match(/^(北緯|南緯|東経|西経)\s*/)
  if (prefixMatch) {
    if (direction) return undefined
    direction = directionFromJapanesePrefix(prefixMatch[1])
    body = body.slice(prefixMatch[0].length).trim()
  }

  if (direction && !isAllowedDirection(direction, axis)) return undefined

  const decimalMatch = body.match(DECIMAL_COORDINATE_PATTERN)
  if (decimalMatch) {
    const parsed = Number(decimalMatch[1])
    if (!Number.isFinite(parsed)) return undefined
    if (direction && parsed < 0) return undefined

    const coordinate = direction
      ? Math.abs(parsed) * directionSign(direction)
      : parsed
    return isValidCoordinate(coordinate, axis) ? coordinate : undefined
  }

  const dmsMatch = body.match(DMS_COORDINATE_PATTERN)
  if (!dmsMatch) return undefined

  const degrees = Number(dmsMatch[1])
  const minutes = Number(dmsMatch[2])
  const seconds = Number(dmsMatch[3])
  if (
    !Number.isFinite(degrees) ||
    !Number.isFinite(minutes) ||
    !Number.isFinite(seconds) ||
    minutes < 0 ||
    minutes >= 60 ||
    seconds < 0 ||
    seconds >= 60
  ) {
    return undefined
  }
  if (direction && degrees < 0) return undefined

  const magnitude = Math.abs(degrees) + minutes / 60 + seconds / 3600
  const coordinate = direction
    ? magnitude * directionSign(direction)
    : Math.sign(degrees) * magnitude
  return isValidCoordinate(coordinate, axis) ? coordinate : undefined
}

export function parseCoordinateParts(
  parts: SexagesimalParts,
  axis: CoordinateAxis,
): number | undefined {
  if (!parts.degrees.trim() || !parts.minutes.trim() || !parts.seconds.trim()) {
    return undefined
  }
  return parseCoordinate(
    `${parts.degrees}°${parts.minutes}′${parts.seconds}″`,
    axis,
  )
}

export function coordinateToSexagesimalParts(
  coordinate: number | undefined,
): SexagesimalParts {
  if (coordinate === undefined || !Number.isFinite(coordinate)) {
    return { degrees: "", minutes: "", seconds: "" }
  }

  const totalCentiseconds = Math.round(Math.abs(coordinate) * 3600 * 100)
  const degrees = Math.floor(totalCentiseconds / (3600 * 100))
  const minutes = Math.floor((totalCentiseconds % (3600 * 100)) / (60 * 100))
  const seconds = (totalCentiseconds % (60 * 100)) / 100

  return {
    degrees: `${coordinate < 0 ? "-" : ""}${degrees}`,
    minutes: minutes.toString(),
    seconds: seconds.toFixed(2),
  }
}

export function formatCoordinateSexagesimal(
  coordinate: number | undefined,
  axis: CoordinateAxis,
): string | undefined {
  if (coordinate === undefined || !Number.isFinite(coordinate)) return undefined

  const parts = coordinateToSexagesimalParts(coordinate)
  const direction =
    axis === "latitude"
      ? coordinate < 0
        ? "S"
        : "N"
      : coordinate < 0
        ? "W"
        : "E"

  return `${parts.degrees.replace(/^-/, "")}°${parts.minutes.padStart(2, "0")}′${parts.seconds.padStart(5, "0")}″${direction}`
}
