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
  const [simulationRates, setSimulationRates] = useState<{ ascent: number; descent: number } | null>(null)

  const handlePredict = useCallback(async (values: PredictorFormValues) => {
    setProgress({ stage: "preparing" })
    setPredictionData(null)
    setMonteCarloData(null)
    setSelectedPointIndex(null)
    setSimulationRates({
      ascent: (values.startInDescent ? 5 : Number(values.ascentRate)),
      descent: Number(values.descentRate),
    })

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

      if (values.weatherSource === "gefs") {
        const result = await invoke<MonteCarloResult>("run_gefs_simulation", {
          startInDescent: values.startInDescent,
          launchLat: values.launchLat,
          launchLon: values.launchLon,
          launchAlt: Number(values.launchAltitude),
          launchTime: launchDateTime.toISOString(),
          ascentRate: (values.startInDescent ? 5 : Number(values.ascentRate)),
          ascentRateStd: (values.startInDescent ? 0 : Number(values.ascentRateStd)),
          grossMassKg: (values.startInDescent ? 6000 : Number(values.totalWeight)) / 1000,
          balloonClassG: (values.startInDescent ? 2000 : Number(values.balloonClass)),
          descentRate: Number(values.descentRate),
          descentRateStd: Number(values.descentRateStd),
          burstAltitudeMean: (values.startInDescent ? Number(values.launchAltitude) : Number(values.burstAltitude)),
          burstAltitudeStd: (values.startInDescent ? 0 : Number(values.burstAltitudeStd)),
          numMembers: Number(values.gefsNumMembers),
          numSamples: Number(values.numSamples),
          demSource: values.demSource,
          openTopoBaseUrl: values.openTopoBaseUrl,
        })
        console.log("GEFS ensemble result:", result)
        setMonteCarloData(result)
      } else if (values.monteCarloEnabled) {
        const result = await invoke<MonteCarloResult>("run_monte_carlo", {
          startInDescent: values.startInDescent,
          launchLat: values.launchLat,
          launchLon: values.launchLon,
          launchAlt: Number(values.launchAltitude),
          launchTime: launchDateTime.toISOString(),
          ascentRate: (values.startInDescent ? 5 : Number(values.ascentRate)),
          ascentRateStd: (values.startInDescent ? 0 : Number(values.ascentRateStd)),
          grossMassKg: (values.startInDescent ? 6000 : Number(values.totalWeight)) / 1000,
          balloonClassG: (values.startInDescent ? 2000 : Number(values.balloonClass)),
          descentRate: Number(values.descentRate),
          descentRateStd: Number(values.descentRateStd),
          burstAltitudeMean: (values.startInDescent ? Number(values.launchAltitude) : Number(values.burstAltitude)),
          burstAltitudeStd: (values.startInDescent ? 0 : Number(values.burstAltitudeStd)),
          numSamples: Number(values.numSamples),
          demSource: values.demSource,
          openTopoBaseUrl: values.openTopoBaseUrl,
        })
        console.log("Monte Carlo result:", result)
        setMonteCarloData(result)
      } else {
        const result = await invoke<PredictionData>("run_simulation", {
          startInDescent: values.startInDescent,
          launchLat: values.launchLat,
          launchLon: values.launchLon,
          launchAlt: Number(values.launchAltitude),
          launchTime: launchDateTime.toISOString(),
          ascentRate: (values.startInDescent ? 5 : Number(values.ascentRate)),
          grossMassKg: (values.startInDescent ? 6000 : Number(values.totalWeight)) / 1000,
          balloonClassG: (values.startInDescent ? 2000 : Number(values.balloonClass)),
          descentRate: Number(values.descentRate),
          burstAltitude: (values.startInDescent ? Number(values.launchAltitude) : Number(values.burstAltitude)),
          demSource: values.demSource,
          openTopoBaseUrl: values.openTopoBaseUrl,
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
          ascentRate={simulationRates?.ascent}
          descentRate={simulationRates?.descent}
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
