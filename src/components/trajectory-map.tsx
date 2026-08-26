import { Map, Marker, useMap } from "@vis.gl/react-maplibre"
import { MapboxOverlay } from "@deck.gl/mapbox"
import { LineLayer, PathLayer, ScatterplotLayer } from "@deck.gl/layers"
import maplibregl from "maplibre-gl"
import { useMemo, useEffect, useCallback, useState, useRef } from "react"
import { TrajectorySummary } from "@/components/trajectory-summary"
import {
  findDefaultSelectedIndex,
  hasSigma,
  pointFillColor,
} from "@/lib/mc-points"
import type {
  MonteCarloPoint,
  MonteCarloResult,
  PredictionData,
  TrajectoryPoint,
} from "@/types"
import "../maplibre-gl.css"

function tinyCirclePath(
  lon: number,
  lat: number,
  alt: number,
  radiusMeters: number,
): [number, number, number][] {
  const mPerDegLat = 111320
  const mPerDegLon = mPerDegLat * Math.cos((lat * Math.PI) / 180)
  const pts: [number, number, number][] = []
  for (let i = 0; i < 24; i++) {
    const a = (i / 24) * Math.PI * 2
    pts.push([
      lon + (radiusMeters * Math.cos(a)) / mPerDegLon,
      lat + (radiusMeters * Math.sin(a)) / mPerDegLat,
      alt,
    ])
  }
  return pts
}

interface TrajectoryMapProps {
  predictionData: PredictionData | null
  monteCarloData?: MonteCarloResult | null
  selectedPointIndex?: number | null
  onPointSelect?: (index: number | null) => void
  launchLat?: number
  launchLon?: number
  mapSelectionMode?: boolean
  onMapClick?: (lat: number, lon: number) => void
}

function DeckGLOverlay({ layers, interleaved }: { layers: any[]; interleaved?: boolean }) {
  const { current: map } = useMap()
  const overlayRef = useRef<MapboxOverlay | null>(null)

  useEffect(() => {
    if (!map) return
    const overlay = new MapboxOverlay({ layers, interleaved, getCursor: () => "" })
    map.addControl(overlay as any)
    overlayRef.current = overlay
    return () => {
      map.removeControl(overlay as any)
      overlayRef.current = null
    }
  }, [map])

  useEffect(() => {
    overlayRef.current?.setProps({ layers })
  }, [layers])

  return null
}

function MapController({
  trajectoryPoints,
  monteCarloPoints,
  cursorValue,
}: {
  trajectoryPoints: TrajectoryPoint[] | null
  monteCarloPoints?: MonteCarloPoint[] | null
  cursorValue: string
}) {
  const { current: map } = useMap()

  useEffect(() => {
    if (!map) return
    const canvasContainer = map.getContainer().querySelector<HTMLElement>('.maplibregl-canvas-container')
    if (canvasContainer) {
      canvasContainer.style.cursor = cursorValue
    }
  }, [map, cursorValue])

  useEffect(() => {
    if (!map) return

    if (monteCarloPoints && monteCarloPoints.length > 0) {
      const coords: [number, number][] = monteCarloPoints.map((p) => [
        p.landing_lon,
        p.landing_lat,
      ])
      if (coords.length === 0) return

      const bounds = coords.reduce(
        (b, c) => b.extend(c),
        new maplibregl.LngLatBounds(coords[0], coords[0]),
      )
      map.fitBounds(bounds, { padding: 60, duration: 1200 })
    } else if (trajectoryPoints && trajectoryPoints.length > 0) {
      const coords: [number, number][] = trajectoryPoints.map((p) => [
        p.lon,
        p.lat,
      ])
      if (coords.length === 0) return

      const bounds = coords.reduce(
        (b, c) => b.extend(c),
        new maplibregl.LngLatBounds(coords[0], coords[0]),
      )
      map.fitBounds(bounds, { padding: 60, duration: 1200 })
    }
  }, [map, trajectoryPoints, monteCarloPoints])

  return null
}

export function TrajectoryMap({
  predictionData,
  monteCarloData = null,
  selectedPointIndex = null,
  onPointSelect,
  launchLat,
  launchLon,
  mapSelectionMode = false,
  onMapClick,
}: TrajectoryMapProps) {
  const [hoveredPointIndex, setHoveredPointIndex] = useState<number | null>(null)
  const [chartFocusPoint, setChartFocusPoint] = useState<
    (TrajectoryPoint & { leg: "ascent" | "descent" }) | null
  >(null)

  const handleChartFocus = useCallback(
    (point: (TrajectoryPoint & { leg: "ascent" | "descent" }) | null) => {
      setChartFocusPoint((prev) => {
        if (prev === null && point === null) return prev
        if (
          prev !== null &&
          point !== null &&
          prev.lat === point.lat &&
          prev.lon === point.lon &&
          prev.alt === point.alt &&
          prev.leg === point.leg
        ) {
          return prev
        }
        return point
      })
    },
    [],
  )

  const handleMapClick = useCallback((e: any) => {
    if (!mapSelectionMode || !onMapClick) return
    const { lat, lng } = e.lngLat
    onMapClick(lat, lng)
  }, [mapSelectionMode, onMapClick])

  const hasLaunchPos =
    launchLat != null && launchLon != null &&
    !isNaN(launchLat) && !isNaN(launchLon) &&
    launchLat >= -90 && launchLat <= 90 &&
    launchLon >= -180 && launchLon <= 180

  const allPoints = useMemo(() => {
    if (monteCarloData) {
      const ascent: TrajectoryPoint[] = monteCarloData.mean_ascent_path ?? []
      const descent: TrajectoryPoint[] = monteCarloData.mean_descent_path ?? []
      return [...ascent, ...descent.slice(1)]
    }
    if (!predictionData) return null
    const ascent: TrajectoryPoint[] = predictionData.ascent_path ?? []
    const descent: TrajectoryPoint[] = predictionData.descent_path ?? []
    return [...ascent, ...descent.slice(1)]
  }, [predictionData, monteCarloData])

  const activeSelectedIndex = useMemo(() => {
    if (selectedPointIndex !== null && selectedPointIndex !== undefined) {
      return selectedPointIndex
    }
    return monteCarloData
      ? findDefaultSelectedIndex(monteCarloData.points)
      : null
  }, [selectedPointIndex, monteCarloData])

  const useSigmaColors = useMemo(
    () => (monteCarloData ? hasSigma(monteCarloData.points) : false),
    [monteCarloData],
  )

  const burstPositions = useMemo(() => {
    function findPeak(pts: TrajectoryPoint[]) {
      if (pts.length === 0) return null
      let peak = pts[0]
      for (const p of pts) {
        if (p.alt > peak.alt) peak = p
      }
      return { lat: peak.lat, lon: peak.lon, alt: peak.alt }
    }

    const result: { lat: number; lon: number; alt: number; isHovered: boolean }[] = []

    const activePeak = (() => {
      if (activeSelectedIndex !== null && activeSelectedIndex !== undefined && monteCarloData?.trajectories?.[activeSelectedIndex]) {
        const traj = monteCarloData.trajectories[activeSelectedIndex]
        return findPeak([...(traj.ascent_path ?? []), ...(traj.descent_path ?? [])])
      }
      return findPeak(allPoints ?? [])
    })()

    if (activePeak) {
      result.push({ ...activePeak, isHovered: false })
    }

    if (
      hoveredPointIndex !== null &&
      hoveredPointIndex !== undefined &&
      hoveredPointIndex !== activeSelectedIndex &&
      monteCarloData?.trajectories?.[hoveredPointIndex]
    ) {
      const traj = monteCarloData.trajectories[hoveredPointIndex]
      const hoverPeak = findPeak([...(traj.ascent_path ?? []), ...(traj.descent_path ?? [])])
      if (hoverPeak) {
        result.push({ ...hoverPeak, isHovered: true })
      }
    }

    return result
  }, [allPoints, monteCarloData, hoveredPointIndex, activeSelectedIndex])

  const deckLayers = useMemo(() => {
    const layers: any[] = []

    if (monteCarloData && monteCarloData.points.length > 0) {
      if (
        hoveredPointIndex !== null && 
        hoveredPointIndex !== undefined && 
        hoveredPointIndex !== activeSelectedIndex
      ) {
        const hoverTraj = monteCarloData.trajectories?.[hoveredPointIndex]
        if (hoverTraj) {
          const hoverAscent: [number, number, number][] = (hoverTraj.ascent_path ?? []).map((p) => [p.lon, p.lat, p.alt])
          const hoverDescent: [number, number, number][] = (hoverTraj.descent_path ?? []).map((p) => [p.lon, p.lat, p.alt])

          if (hoverAscent.length >= 2) {
            layers.push(
              new PathLayer({
                id: "mc-hovered-ascent",
                data: [{ path: hoverAscent }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [255, 100, 100, 80],
                widthUnits: "pixels",
                getWidth: 4,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
            layers.push(
              new PathLayer({
                id: "mc-hovered-ascent-ground",
                data: [{ path: hoverAscent.map(([lon, lat]) => [lon, lat, 0]) }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [255, 100, 100, 40],
                widthUnits: "pixels",
                getWidth: 0.75,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
          }

          if (hoverDescent.length >= 2) {
            layers.push(
              new PathLayer({
                id: "mc-hovered-descent",
                data: [{ path: hoverDescent }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [100, 160, 255, 80],
                widthUnits: "pixels",
                getWidth: 4,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
            layers.push(
              new PathLayer({
                id: "mc-hovered-descent-ground",
                data: [{ path: hoverDescent.map(([lon, lat]) => [lon, lat, 0]) }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [100, 160, 255, 40],
                widthUnits: "pixels",
                getWidth: 0.75,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
          }
        }
      }

      if (activeSelectedIndex !== null && activeSelectedIndex !== undefined) {
        const traj = monteCarloData.trajectories?.[activeSelectedIndex]
        if (traj) {
          const selAscent: [number, number, number][] = (traj.ascent_path ?? []).map((p) => [p.lon, p.lat, p.alt])
          const selDescent: [number, number, number][] = (traj.descent_path ?? []).map((p) => [p.lon, p.lat, p.alt])

          if (selAscent.length >= 2) {
            layers.push(
              new PathLayer({
                id: "mc-selected-ascent",
                data: [{ path: selAscent }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [255, 100, 100, 240],
                widthUnits: "pixels",
                getWidth: 5,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
            layers.push(
              new PathLayer({
                id: "mc-selected-ascent-ground",
                data: [{ path: selAscent.map(([lon, lat]) => [lon, lat, 0]) }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [255, 100, 100, 240],
                widthUnits: "pixels",
                getWidth: 0.75,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
          }

          if (selDescent.length >= 2) {
            layers.push(
              new PathLayer({
                id: "mc-selected-descent",
                data: [{ path: selDescent }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [100, 160, 255, 240],
                widthUnits: "pixels",
                getWidth: 5,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
            layers.push(
              new PathLayer({
                id: "mc-selected-descent-ground",
                data: [{ path: selDescent.map(([lon, lat]) => [lon, lat, 0]) }],
                getPath: (d: { path: [number, number, number][] }) => d.path,
                getColor: [100, 160, 255, 240],
                widthUnits: "pixels",
                getWidth: 0.75,
                billboard: true,
                rounded: true,
                jointRounded: true,
              }),
            )
          }
        }
      }

      if (monteCarloData.points.length > 1) {
        layers.push(
          new ScatterplotLayer({
            id: "monte-carlo-landing-points",
            data: monteCarloData.points,
            getPosition: (d: MonteCarloPoint) => [d.landing_lon, d.landing_lat],
            getFillColor: (_d: MonteCarloPoint, { index }: { index: number }) =>
              activeSelectedIndex === index
                ? [255, 255, 255, 255]
                : pointFillColor(_d, useSigmaColors),
            getLineColor: (_d: MonteCarloPoint, { index }: { index: number }) =>
              activeSelectedIndex === index
                ? [0, 0, 0, 255]
                : [255, 255, 255, 220],
            getRadius: (_d: MonteCarloPoint, { index }: { index: number }) =>
              activeSelectedIndex === index ? 120 : 80,
            radiusUnits: "meters",
            radiusMinPixels: 6,
            radiusMaxPixels: 6,
            stroked: true,
            lineWidthMinPixels: 0,
            lineWidthMaxPixels: 3,
            billboard: true,
            pickable: true,
            onHover: (info: any) => {
              console.log("onHover", info.object ? "object" : "null", info.index)
              if (info.object && monteCarloData) {
                const idx = monteCarloData.points.indexOf(info.object)
                setHoveredPointIndex(idx !== -1 ? idx : null)
              } else {
                setHoveredPointIndex(null)
              }
            },
            onClick: (info: any) => {
              console.log("onClick", info.index, "activeSelectedIndex", activeSelectedIndex)
              if (info.index !== undefined && info.index !== -1) {
                onPointSelect?.(info.index === activeSelectedIndex ? null : info.index)
              }
            },
            onDrag: () => {},
          }),
        )
      }

    } else if (predictionData) {
      const ascentPath: [number, number, number][] = (
        predictionData.ascent_path ?? []
      ).map((p: TrajectoryPoint) => [p.lon, p.lat, p.alt])
      const descentPath: [number, number, number][] = (
        predictionData.descent_path ?? []
      ).map((p: TrajectoryPoint) => [p.lon, p.lat, p.alt])

      if (ascentPath.length >= 2) {
        layers.push(
          new PathLayer({
            id: "ascent-line-3d",
            data: [{ path: ascentPath }],
            getPath: (d: { path: [number, number, number][] }) => d.path,
            getColor: [239, 68, 68, 255],
            widthUnits: "pixels",
            getWidth: 4,
            billboard: true,
            rounded: true,
            jointRounded: true,
          }),
        )
        layers.push(
          new PathLayer({
            id: "ascent-line-ground",
            data: [{ path: ascentPath.map(([lon, lat]) => [lon, lat, 0]) }],
            getPath: (d: { path: [number, number, number][] }) => d.path,
            getColor: [239, 68, 68, 255],
            widthUnits: "pixels",
            getWidth: 0.75,
            billboard: true,
            rounded: true,
            jointRounded: true,
          }),
        )
      }

      if (descentPath.length >= 2) {
        layers.push(
          new PathLayer({
            id: "descent-line-3d",
            data: [{ path: descentPath }],
            getPath: (d: { path: [number, number, number][] }) => d.path,
            getColor: [59, 130, 246, 255],
            widthUnits: "pixels",
            getWidth: 4,
            billboard: true,
            rounded: true,
            jointRounded: true,
          }),
        )
        layers.push(
          new PathLayer({
            id: "descent-line-ground",
            data: [{ path: descentPath.map(([lon, lat]) => [lon, lat, 0]) }],
            getPath: (d: { path: [number, number, number][] }) => d.path,
            getColor: [59, 130, 246, 255],
            widthUnits: "pixels",
            getWidth: 0.75,
            billboard: true,
            rounded: true,
            jointRounded: true,
          }),
        )
      }
    }

    if (burstPositions.length > 0) {
      for (let i = 0; i < burstPositions.length; i++) {
        const bp = burstPositions[i]
        layers.push(
          new PathLayer({
            id: `burst-point-circle-${i}`,
            data: [{ path: tinyCirclePath(bp.lon, bp.lat, bp.alt, 20) }],
            getPath: (d: { path: [number, number, number][] }) => d.path,
            getColor: bp.isHovered ? [255, 200, 0, 180] : [255, 215, 0, 255],
            widthUnits: "pixels",
            getWidth: bp.isHovered ? 4 : 6,
            billboard: true,
            rounded: true,
            jointRounded: true,
            pickable: false,
          }),
        )
      }
    }

    if (chartFocusPoint) {
      const { lon, lat, alt } = chartFocusPoint
      layers.push(
        new LineLayer({
          id: "chart-focus-leader",
          data: [{ source: [lon, lat, 0], target: [lon, lat, alt] }],
          getSourcePosition: (d: { source: [number, number, number] }) => d.source,
          getTargetPosition: (d: { target: [number, number, number] }) => d.target,
          getColor: [234, 179, 8, 160],
          getWidth: 4,
          widthUnits: "pixels",
          getDashArray: [6, 4],
        }),
      )
      const dotColor: [number, number, number, number] = chartFocusPoint.leg === "ascent"
        ? [239, 68, 68, 255]
        : [59, 130, 246, 255]
      layers.push(
        new PathLayer({
          id: "chart-focus-dot",
          data: [{ path: tinyCirclePath(lon, lat, alt, 10) }],
          getPath: (d: { path: [number, number, number][] }) => d.path,
          getColor: dotColor,
          widthUnits: "pixels",
          getWidth: 8,
          billboard: true,
          rounded: true,
          jointRounded: true,
          pickable: false,
        }),
      )
    }

    return layers
  }, [predictionData, monteCarloData, activeSelectedIndex, hoveredPointIndex, chartFocusPoint, burstPositions, onPointSelect])

  const landingPos = useMemo(() => {
    if (monteCarloData) return null
    if (!predictionData) return null
    return { lat: predictionData.landing_lat, lon: predictionData.landing_lon }
  }, [predictionData, monteCarloData])

  const monteCarloPoints = useMemo(() => {
    return monteCarloData?.points ?? null
  }, [monteCarloData])

  const mapCursor = useMemo(() => {
    if (mapSelectionMode) return "crosshair"
    if (hoveredPointIndex !== null) return "pointer"
    return "default"
  }, [mapSelectionMode, hoveredPointIndex])

  return (
    <div className="w-full h-full">
      <Map
        initialViewState={{
          longitude: 135,
          latitude: 35,
          zoom: 5,
          pitch: 60,
        }}
        maxPitch={85}
        style={{ width: "100%", height: "100%" }}
        mapStyle="https://tiles.openfreemap.org/styles/liberty"
        minZoom={2}
        onClick={handleMapClick}
      >
        <MapController
          trajectoryPoints={allPoints}
          monteCarloPoints={monteCarloPoints}
          cursorValue={mapCursor}
        />

        <DeckGLOverlay 
          layers={deckLayers} 
          interleaved={true} 
        />

        {hasLaunchPos && (
          <Marker
            longitude={launchLon!}
            latitude={launchLat!}
            anchor="bottom"
          >
            <div className="relative cursor-pointer">
              <div className="w-3 h-3 bg-red-500 border-2 border-white rounded-full shadow-lg" />
              <span className="absolute -top-5 left-1/2 -translate-x-1/2 text-[10px] font-bold bg-background/80 px-1 rounded whitespace-nowrap">
                放球
              </span>
            </div>
          </Marker>
        )}

        {landingPos && (
          <Marker
            longitude={landingPos.lon}
            latitude={landingPos.lat}
            anchor="bottom"
          >
            <div className="relative cursor-pointer">
              <div className="w-3 h-3 bg-blue-500 border-2 border-white rounded-full shadow-lg" />
              <span className="absolute -top-5 left-1/2 -translate-x-1/2 text-[10px] font-bold bg-background/80 px-1 rounded whitespace-nowrap">
                着地
              </span>
            </div>
          </Marker>
        )}
      </Map>

      {monteCarloData && monteCarloData.points.length > 0 && (
        <div className="absolute top-4 right-4 z-10 bg-sidebar/95 text-sidebar-foreground rounded-lg p-3 text-xs space-y-1.5 shadow-sm ring-1 ring-sidebar-border">
          <p className="font-medium text-[11px] mb-1">偏差 (σ)</p>
          <div className="flex items-center gap-2">
            <span className="w-3 h-3 rounded-full bg-[rgb(34,197,94)]" />
            <span>±1σ 以内</span>
          </div>
          <div className="flex items-center gap-2">
            <span className="w-3 h-3 rounded-full bg-[rgb(234,179,8)]" />
            <span>±1σ〜2σ</span>
          </div>
          <div className="flex items-center gap-2">
            <span className="w-3 h-3 rounded-full bg-[rgb(239,68,68)]" />
            <span>±2σ 超</span>
          </div>
        </div>
      )}

      <TrajectorySummary
        predictionData={predictionData}
        monteCarloData={monteCarloData}
        selectedPointIndex={selectedPointIndex}
        launchLat={launchLat}
        launchLon={launchLon}
        onPointHover={handleChartFocus}
      />
    </div>
  )
}
