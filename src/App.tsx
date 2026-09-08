import { SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar"
import { AppSidebar, PredictorFormValues, PositionMode, PRESETS, ProgressInfo } from "@/components/app-sidebar"
import { TrajectoryMap } from "@/components/trajectory-map"
import { useState, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import type { MonteCarloResult, PredictionData } from "@/types"

const DEFAULT_LAT = PRESETS[0].lat
const DEFAULT_LON = PRESETS[0].lon

function App() {
  const [predictionData, setPredictionData] = useState<PredictionData | null>(null)
  const [monteCarloData, setMonteCarloData] = useState<MonteCarloResult | null>(null)
  const [selectedPointIndex, setSelectedPointIndex] = useState<number | null>(null)
  const [progress, setProgress] = useState<ProgressInfo | null>(null)
  const [positionMode, setPositionMode] = useState<PositionMode>("preset")
  const [launchLat, setLaunchLat] = useState<number>(DEFAULT_LAT)
  const [launchLon, setLaunchLon] = useState<number>(DEFAULT_LON)
  const [launchTimeUtc, setLaunchTimeUtc] = useState<string | null>(null)

  const handlePredict = useCallback(async (values: PredictorFormValues) => {
    setProgress({ stage: "preparing" })
    setPredictionData(null)
    setMonteCarloData(null)
    setSelectedPointIndex(null)

    const unlisten = await listen<ProgressInfo>("progress", (event) => {
      setProgress(event.payload)
    })

    try {
      const launchDate = values.launchDate!
      const [hours, minutes] = values.launchTime.split(":").map(Number)
      // 入力はJST壁時計として解釈し、UTCに変換する（-9時間）
      const launchDateTime = new Date(
        Date.UTC(
          launchDate.getFullYear(),
          launchDate.getMonth(),
          launchDate.getDate(),
          hours - 9,
          minutes,
          0,
          0,
        ),
      )
      setLaunchTimeUtc(launchDateTime.toISOString())

      const gfsRunTime = new Date(
        launchDateTime.getTime() - 12 * 60 * 60 * 1000,
      )

      if (values.weatherSource === "gefs") {
        const result = await invoke<MonteCarloResult>("run_gefs_simulation", {
          launchLat: values.launchLat,
          launchLon: values.launchLon,
          launchAlt: 10.0,
          gefsRunTime: gfsRunTime.toISOString(),
          launchTime: launchDateTime.toISOString(),
          ascentRate: Number(values.ascentRate),
          grossMassKg: Number(values.totalWeight) / 1000,
          balloonClassG: Number(values.balloonClass),
          descentRate: Number(values.descentRate),
          burstAltitudeMean: Number(values.burstAltitude),
          burstAltitudeStd: Number(values.burstAltitudeStd),
          numMembers: Number(values.gefsNumMembers),
          numSamples: Number(values.numSamples),
        })
        console.log("GEFS ensemble result:", result)
        setMonteCarloData(result)
      } else if (values.monteCarloEnabled) {
        const result = await invoke<MonteCarloResult>("run_monte_carlo", {
          launchLat: values.launchLat,
          launchLon: values.launchLon,
          launchAlt: 10.0,
          gfsRunTime: gfsRunTime.toISOString(),
          launchTime: launchDateTime.toISOString(),
          ascentRate: Number(values.ascentRate),
          grossMassKg: Number(values.totalWeight) / 1000,
          balloonClassG: Number(values.balloonClass),
          descentRate: Number(values.descentRate),
          burstAltitudeMean: Number(values.burstAltitude),
          burstAltitudeStd: Number(values.burstAltitudeStd),
          numSamples: Number(values.numSamples),
        })
        console.log("Monte Carlo result:", result)
        setMonteCarloData(result)
      } else {
        const result = await invoke<PredictionData>("run_simulation", {
          launchLat: values.launchLat,
          launchLon: values.launchLon,
          launchAlt: 10.0,
          gfsRunTime: gfsRunTime.toISOString(),
          launchTime: launchDateTime.toISOString(),
          ascentRate: Number(values.ascentRate),
          grossMassKg: Number(values.totalWeight) / 1000,
          balloonClassG: Number(values.balloonClass),
          descentRate: Number(values.descentRate),
          burstAltitude: Number(values.burstAltitude),
        })
        console.log("Simulation result:", result)
        setPredictionData(result)
      }
    } catch (e) {
      console.error("Simulation failed:", e)
    } finally {
      unlisten()
      setProgress(null)
    }
  }, [])

  const handleLaunchPositionChange = (lat: number, lon: number) => {
    setLaunchLat(lat)
    setLaunchLon(lon)
  }

  return (
    <SidebarProvider>
      <AppSidebar
        onSubmit={handlePredict}
        progress={progress}
        launchLat={launchLat}
        launchLon={launchLon}
        onLaunchPositionChange={handleLaunchPositionChange}
        positionMode={positionMode}
        onPositionModeChange={setPositionMode}
      />
      <main className="absolute inset-0 w-screen h-screen z-0">
        <div className="absolute top-4 left-4 z-20">
          <SidebarTrigger className="bg-sidebar text-sidebar-foreground shadow-sm ring-1 ring-sidebar-border" />
        </div>
        <TrajectoryMap
          predictionData={predictionData}
          monteCarloData={monteCarloData}
          selectedPointIndex={selectedPointIndex}
          onPointSelect={setSelectedPointIndex}
          launchLat={launchLat}
          launchLon={launchLon}
          launchTimeUtc={launchTimeUtc}
          mapSelectionMode={positionMode === "map"}
          onMapClick={(lat, lon) => {
            setLaunchLat(lat)
            setLaunchLon(lon)
            setPositionMode("preset")
          }}
        />
      </main>
    </SidebarProvider>
  )
}

export default App