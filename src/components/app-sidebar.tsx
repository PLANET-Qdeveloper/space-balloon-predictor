import { useForm } from "@tanstack/react-form"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { Balloon, CalendarDays, ChevronsDown, ChevronsUp, Clock, Crosshair, Dice5, Layers, Loader, MapPin, Pencil, Play, Sigma, Weight } from "lucide-react"
import { InputGroup, InputGroupAddon, InputGroupInput, InputGroupText } from "@/components/ui/input-group"
import {
  Field,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"

import { Calendar } from "@/components/ui/calendar"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useState, useEffect } from "react"

export type PositionMode = "preset" | "custom" | "map"

export type WeatherSource = "gfs" | "gefs"

export interface PredictorFormValues {
  launchLat: number
  launchLon: number
  launchDate: Date | undefined
  launchTime: string
  balloonClass: string
  totalWeight: string
  ascentRate: string
  descentRate: string
  burstAltitude: string
  monteCarloEnabled: boolean
  burstAltitudeStd: string
  numSamples: string
  weatherSource: WeatherSource
  gefsNumMembers: string
}

export type ProgressInfo =
  | { stage: "preparing" }
  | { stage: "downloading_gfs"; current: number; total: number }
  | { stage: "downloading_gefs"; current: number; total: number; member: string }
  | { stage: "decoding_grib" }
  | { stage: "running_simulation" }
  | { stage: "running_monte_carlo"; current: number; total: number }

interface AppSidebarProps {
  onSubmit: (values: PredictorFormValues) => void | Promise<void>
  progress?: ProgressInfo | null
  launchLat: number
  launchLon: number
  onLaunchPositionChange: (lat: number, lon: number) => void
  positionMode: PositionMode
  onPositionModeChange: (mode: PositionMode) => void
}

export const PRESETS = [
  { name: "伊都キャンパス", lat: 33.5969, lon: 130.2236 },
  { name: "南レク南楽園ファミリーパーク", lat: 33.13492, lon: 132.50477 },
  { name: "津島プレーランド", lat: 33.12604, lon: 132.50333 },
  { name: "須ノ川公園キャンプ場", lat: 33.04107, lon: 132.48698 },
  { name: "南レク松軒山公園", lat: 32.97239, lon: 132.55583 },
  { name: "南レク御荘公園", lat: 32.96417, lon: 132.55206 },
  { name: "南レク城辺公園", lat: 32.95287, lon: 132.58429 },
  { name: "土佐西南大規模公園", lat: 33.02483, lon: 133.01651 },
] as const

const DEFAULT_LAT = PRESETS[0].lat
const DEFAULT_LON = PRESETS[0].lon

const validateTime24h = (value: string) => {
  if (!value) return "時刻を入力してください"
  const regex = /^([01]?[0-9]|2[0-3]):[0-5][0-9]$/
  if (!regex.test(value)) return "24時間形式 (例: 13:00) で入力してください"
  return undefined
}

const validatePositiveNumber = (value: string, label: string) => {
  if (!value) return `${label}を入力してください`
  const num = Number(value)
  if (isNaN(num)) return "数値で入力してください"
  if (num <= 0) return "0より大きい数値を入力してください"
  return undefined
}

function findPreset(lat: number, lon: number) {
  return PRESETS.find(p => Math.abs(p.lat - lat) < 0.0001 && Math.abs(p.lon - lon) < 0.0001)
}

function formatProgressText(progress: ProgressInfo): string {
  switch (progress.stage) {
    case "preparing":
      return "準備中..."
    case "downloading_gfs":
      return `気象データ取得中 (${progress.current}/${progress.total})...`
    case "downloading_gefs":
      return `気象データ取得中 (${progress.current}/${progress.total})...`
    case "decoding_grib":
      return "気象データを解析中..."
    case "running_simulation":
      return "シミュレーション実行中..."
    case "running_monte_carlo":
      return `シミュレーション実行中 (${progress.current}/${progress.total})...`
    default:
      return "処理中..."
  }
}

export function AppSidebar({
  onSubmit,
  progress = null,
  launchLat,
  launchLon,
  onLaunchPositionChange,
  positionMode,
  onPositionModeChange,
}: AppSidebarProps) {
  const [popoverOpen, setPopoverOpen] = useState(false)

  const defaultValues: PredictorFormValues = {
    launchLat: launchLat ?? DEFAULT_LAT,
    launchLon: launchLon ?? DEFAULT_LON,
    launchDate: undefined,
    launchTime: "",
    balloonClass: "2000",
    totalWeight: "6000",
    ascentRate: "6",
    descentRate: "6",
    burstAltitude: "30000",
    monteCarloEnabled: false,
    burstAltitudeStd: "1000",
    numSamples: "50",
    weatherSource: "gfs",
    gefsNumMembers: "31",
  }

  const form = useForm({
    defaultValues,
    onSubmit: async ({ value }) => {
      await onSubmit(value)
    },
  })

  useEffect(() => {
    if (launchLat == null || launchLon == null) return

    const currentLat = form.getFieldValue("launchLat")
    const currentLon = form.getFieldValue("launchLon")

    if (currentLat !== launchLat) {
      form.setFieldValue("launchLat", launchLat)
    }
    if (currentLon !== launchLon) {
      form.setFieldValue("launchLon", launchLon)
    }
  }, [launchLat, launchLon, form])

  const handlePresetSelect = (preset: typeof PRESETS[number]) => {
    onPositionModeChange("preset")
    onLaunchPositionChange(preset.lat, preset.lon)
    setPopoverOpen(false)
  }

  const handleCustomSelect = () => {
    onPositionModeChange("custom")
    setPopoverOpen(false)
  }

  const handleMapSelect = () => {
    onPositionModeChange("map")
    setPopoverOpen(false)
  }

  const matchedPreset = findPreset(launchLat, launchLon)
  const triggerLabel =
    positionMode === "custom"
      ? "直接入力"
      : positionMode === "map"
        ? "地図から選択"
        : (matchedPreset ? matchedPreset.name : "選択済み")

  const scatterFields = (
    <>
      {/* 標準偏差 */}
      <form.Field
        name="burstAltitudeStd"
        validators={{
          onChange: ({ value }) => validatePositiveNumber(value, "標準偏差"),
        }}
        children={(field) => (
          <Field>
            <FieldLabel className="text-xs" htmlFor={field.name}>バースト高度の標準偏差</FieldLabel>
            <InputGroup>
              <InputGroupInput
                id={field.name}
                name={field.name}
                value={field.state.value}
                onBlur={field.handleBlur}
                onChange={(e) => field.handleChange(e.target.value)}
                type="text"
                placeholder="標準偏差を入力"
              />
              <InputGroupAddon align="inline-start">
                <Sigma className="text-muted-foreground" />
              </InputGroupAddon>
              <InputGroupAddon align="inline-end">
                <InputGroupText className="text-muted-foreground">m</InputGroupText>
              </InputGroupAddon>
            </InputGroup>
            {field.state.meta.isTouched && field.state.meta.errors.length ? (
              <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
            ) : null}
          </Field>
        )}
      />

      {/* サンプル数 */}
      <form.Field
        name="numSamples"
        validators={{
          onChange: ({ value }) => {
            if (!value) return "サンプル数を入力してください"
            const num = Number(value)
            if (isNaN(num) || !Number.isInteger(num)) return "整数で入力してください"
            if (num < 1 || num > 200) return "1〜200 の範囲で入力してください"
            return undefined
          },
        }}
        children={(field) => (
          <Field>
            <FieldLabel className="text-xs" htmlFor={field.name}>サンプル数</FieldLabel>
            <InputGroup>
              <InputGroupInput
                id={field.name}
                name={field.name}
                value={field.state.value}
                onBlur={field.handleBlur}
                onChange={(e) => field.handleChange(e.target.value)}
                type="text"
                placeholder="サンプル数を入力"
              />
              <InputGroupAddon align="inline-start">
                <Dice5 className="text-muted-foreground" />
              </InputGroupAddon>
            </InputGroup>
            {field.state.meta.isTouched && field.state.meta.errors.length ? (
              <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
            ) : null}
          </Field>
        )}
      />
    </>
  )

  return (
    <Sidebar variant="floating">
      <form
        onSubmit={(e) => {
          e.preventDefault()
          e.stopPropagation()
          form.handleSubmit()
        }}
        className="flex flex-col h-full"
      >
        <SidebarHeader className="py-4 px-3 flex flex-row items-center gap-2">
          <Balloon />
          <h1 className="font-medium">Space Balloon Predictor</h1>
        </SidebarHeader>

        <SidebarContent className="p-4 *:list-none">
          <SidebarMenuItem className="space-y-6">
            {/* 放球位置 */}
            <Field>
              <FieldLabel className="text-xs">放球位置</FieldLabel>
              <Popover open={popoverOpen} onOpenChange={setPopoverOpen}>
                <PopoverTrigger asChild>
                  <button type="button" className="w-full text-left outline-none block">
                    <InputGroup>
                      <InputGroupInput
                        value={triggerLabel}
                        readOnly
                        className="cursor-pointer placeholder:text-muted-foreground placeholder:text-sm"
                      />
                      <InputGroupAddon align="inline-start">
                        <MapPin className="text-muted-foreground" />
                      </InputGroupAddon>
                    </InputGroup>
                  </button>
                </PopoverTrigger>
                <PopoverContent className="w-[--radix-popover-trigger-width] p-1" align="start">
                  <div className="flex flex-col">
                    {PRESETS.map((preset) => (
                      <button
                        key={preset.name}
                        type="button"
                        onClick={() => handlePresetSelect(preset)}
                        className={`flex items-center gap-2 w-full px-2 py-1.5 text-sm rounded-md text-left hover:bg-accent hover:text-accent-foreground ${positionMode === "preset" && matchedPreset?.name === preset.name ? "bg-accent text-accent-foreground" : ""}`}
                      >
                        <MapPin className="size-3.5 shrink-0 text-muted-foreground" />
                        {preset.name}
                      </button>
                    ))}
                    <div className="my-1 h-px bg-border" />
                    <button
                      type="button"
                      onClick={handleCustomSelect}
                      className={`flex items-center gap-2 w-full px-2 py-1.5 text-sm rounded-md text-left hover:bg-accent hover:text-accent-foreground ${positionMode === "custom" ? "bg-accent text-accent-foreground" : ""}`}
                    >
                      <Pencil className="size-3.5 shrink-0 text-muted-foreground" />
                      直接入力
                    </button>
                    <button
                      type="button"
                      onClick={handleMapSelect}
                      className={`flex items-center gap-2 w-full px-2 py-1.5 text-sm rounded-md text-left hover:bg-accent hover:text-accent-foreground ${positionMode === "map" ? "bg-accent text-accent-foreground" : ""}`}
                    >
                      <Crosshair className="size-3.5 shrink-0 text-muted-foreground" />
                      地図から選択
                    </button>
                  </div>
                </PopoverContent>
              </Popover>

              {positionMode === "custom" && (
                <div className="flex gap-2 mt-1.5">
                  <form.Field
                    name="launchLat"
                    validators={{
                      onChange: ({ value }) => {
                        if (value === undefined || value === null || isNaN(value)) return "緯度を入力してください"
                        if (value < -90 || value > 90) return "緯度は -90〜90 の範囲で入力してください"
                        return undefined
                      },
                    }}
                    children={(latField) => (
                      <div className="flex-1">
                        <InputGroup>
                          <InputGroupInput
                            value={latField.state.value?.toString() ?? ""}
                            onChange={(e) => {
                              const val = e.target.value
                              const num = val === "" ? NaN : parseFloat(val)
                              latField.handleChange(num)
                              if (!isNaN(num) && num >= -90 && num <= 90) {
                                onLaunchPositionChange(num, form.getFieldValue("launchLon"))
                              }
                            }}
                            onBlur={latField.handleBlur}
                            type="number"
                            step="any"
                            placeholder="緯度"
                          />
                          <InputGroupAddon align="inline-start">
                            <span className="text-[10px] text-muted-foreground">緯度</span>
                          </InputGroupAddon>
                        </InputGroup>
                        {latField.state.meta.isTouched && latField.state.meta.errors.length ? (
                          <p className="text-[11px] text-red-500 mt-1">{latField.state.meta.errors.join(", ")}</p>
                        ) : null}
                      </div>
                    )}
                  />

                  <form.Field
                    name="launchLon"
                    validators={{
                      onChange: ({ value }) => {
                        if (value === undefined || value === null || isNaN(value)) return "経度を入力してください"
                        if (value < -180 || value > 180) return "経度は -180〜180 の範囲で入力してください"
                        return undefined
                      },
                    }}
                    children={(lonField) => (
                      <div className="flex-1">
                        <InputGroup>
                          <InputGroupInput
                            value={lonField.state.value?.toString() ?? ""}
                            onChange={(e) => {
                              const val = e.target.value
                              const num = val === "" ? NaN : parseFloat(val)
                              lonField.handleChange(num)
                              if (!isNaN(num) && num >= -180 && num <= 180) {
                                onLaunchPositionChange(form.getFieldValue("launchLat"), num)
                              }
                            }}
                            onBlur={lonField.handleBlur}
                            type="number"
                            step="any"
                            placeholder="経度"
                          />
                          <InputGroupAddon align="inline-start">
                            <span className="text-[10px] text-muted-foreground">経度</span>
                          </InputGroupAddon>
                        </InputGroup>
                        {lonField.state.meta.isTouched && lonField.state.meta.errors.length ? (
                          <p className="text-[11px] text-red-500 mt-1">{lonField.state.meta.errors.join(", ")}</p>
                        ) : null}
                      </div>
                    )}
                  />
                </div>
              )}

              {positionMode === "map" && (
                <p className="text-[11px] text-muted-foreground mt-1.5">
                  地図をクリックして放球位置を選択してください
                </p>
              )}
            </Field>

            <FieldGroup className="flex flex-col gap-y-2">
              {/* 放球日 */}
              <form.Field
                name="launchDate"
                validators={{
                  onChange: ({ value }) => (!value ? "日付を選択してください" : undefined),
                }}
                children={(field) => (
                  <Field className="flex-1">
                    <FieldLabel className="text-xs" htmlFor={field.name}>放球日</FieldLabel>
                    <Popover>
                      <PopoverTrigger asChild>
                        <button type="button" className="w-full text-left outline-none block">
                          <InputGroup>
                            <InputGroupInput
                              id={field.name}
                              name={field.name}
                              value={
                                field.state.value
                                  ? field.state.value.toLocaleDateString("ja-JP", {
                                      year: "numeric",
                                      month: "2-digit",
                                      day: "2-digit",
                                    })
                                  : ""
                              }
                              placeholder="日付を選択"
                              readOnly
                              className="cursor-pointer placeholder:text-muted-foreground placeholder:text-sm"
                            />
                            <InputGroupAddon align="inline-start">
                              <CalendarDays className="text-muted-foreground" />
                            </InputGroupAddon>
                          </InputGroup>
                        </button>
                      </PopoverTrigger>
                      <PopoverContent className="w-auto p-0" align="start">
                        <Calendar
                          mode="single"
                          selected={field.state.value as Date | undefined}
                          onSelect={(date) => {
                            field.handleChange(date as Date)
                            field.handleBlur()
                          }}
                        />
                      </PopoverContent>
                    </Popover>
                    {field.state.meta.isTouched && field.state.meta.errors.length ? (
                      <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                    ) : null}
                  </Field>
                )}
              />

              {/* 放球時刻 */}
              <form.Field
                name="launchTime"
                validators={{
                  onChange: ({ value }) => validateTime24h(value),
                }}
                children={(field) => (
                  <Field className="flex-1">
                    <FieldLabel className="text-xs" htmlFor={field.name}>放球時刻（JST）</FieldLabel>
                    <InputGroup>
                      <InputGroupInput
                        id={field.name}
                        name={field.name}
                        value={field.state.value}
                        onBlur={field.handleBlur}
                        onChange={(e) => field.handleChange(e.target.value)}
                        type="text"
                        placeholder="13:00"
                        className="w-full"
                      />
                      <InputGroupAddon align="inline-start">
                        <Clock className="text-muted-foreground" />
                      </InputGroupAddon>
                    </InputGroup>
                    {field.state.meta.isTouched && field.state.meta.errors.length ? (
                      <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                    ) : null}
                  </Field>
                )}
              />
            </FieldGroup>

            <FieldGroup className="flex flex-col gap-y-2">
              {/* 気球種別 */}
              <form.Field
                name="balloonClass"
                validators={{
                  onChange: ({ value }) => {
                    if (!["1000", "1500", "2000", "3000"].includes(value)) return "気球種別を選択してください"
                    return undefined
                  },
                }}
                children={(field) => (
                  <Field className="flex-1">
                    <FieldLabel className="text-xs" htmlFor={field.name}>気球種別</FieldLabel>
                    <Select
                      value={field.state.value}
                      onValueChange={(val) => field.handleChange(val)}
                    >
                      <SelectTrigger className="w-full mt-1.5" id={field.name}>
                        <SelectValue placeholder="気球を選択" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="1000">1000g</SelectItem>
                        <SelectItem value="1500">1500g</SelectItem>
                        <SelectItem value="2000">2000g</SelectItem>
                        <SelectItem value="3000">3000g</SelectItem>
                      </SelectContent>
                    </Select>
                    {field.state.meta.isTouched && field.state.meta.errors.length ? (
                      <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                    ) : null}
                  </Field>
                )}
              />

              {/* 総重量 */}
              <form.Field
                name="totalWeight"
                validators={{
                  onChange: ({ value }) => validatePositiveNumber(value, "総重量"),
                }}
                children={(field) => (
                  <Field className="flex-1">
                    <FieldLabel className="text-xs" htmlFor={field.name}>総重量</FieldLabel>
                    <InputGroup>
                      <InputGroupInput
                        id={field.name}
                        name={field.name}
                        value={field.state.value}
                        onBlur={field.handleBlur}
                        onChange={(e) => field.handleChange(e.target.value)}
                        type="text"
                        placeholder="総重量（g）"
                      />
                      <InputGroupAddon align="inline-start">
                        <Weight className="text-muted-foreground" />
                      </InputGroupAddon>
                      <InputGroupAddon align="inline-end">
                        <InputGroupText className="text-muted-foreground">g</InputGroupText>
                      </InputGroupAddon>
                    </InputGroup>
                    {field.state.meta.isTouched && field.state.meta.errors.length ? (
                      <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                    ) : null}
                  </Field>
                )}
              />
            </FieldGroup>

            <FieldGroup className="flex flex-col gap-y-2">
              {/* 上昇速度 */}
              <form.Field
                name="ascentRate"
                validators={{
                  onChange: ({ value }) => validatePositiveNumber(value, "目標上昇速度"),
                }}
                children={(field) => (
                  <Field className="flex-1">
                    <FieldLabel className="text-xs" htmlFor={field.name}>目標上昇速度</FieldLabel>
                    <InputGroup>
                      <InputGroupInput
                        id={field.name}
                        name={field.name}
                        value={field.state.value}
                        onBlur={field.handleBlur}
                        onChange={(e) => field.handleChange(e.target.value)}
                        type="text"
                        placeholder="上昇速度"
                      />
                      <InputGroupAddon align="inline-start">
                        <ChevronsUp className="text-muted-foreground" />
                      </InputGroupAddon>
                      <InputGroupAddon align="inline-end">
                        <InputGroupText className="text-muted-foreground">m/s</InputGroupText>
                      </InputGroupAddon>
                    </InputGroup>
                    {field.state.meta.isTouched && field.state.meta.errors.length ? (
                      <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                    ) : null}
                  </Field>
                )}
              />

              {/* 落下速度 */}
              <form.Field
                name="descentRate"
                validators={{
                  onChange: ({ value }) => validatePositiveNumber(value, "落下速度"),
                }}
                children={(field) => (
                  <Field className="flex-1">
                    <FieldLabel className="text-xs" htmlFor={field.name}>落下速度</FieldLabel>
                    <InputGroup>
                      <InputGroupInput
                        id={field.name}
                        name={field.name}
                        value={field.state.value}
                        onBlur={field.handleBlur}
                        onChange={(e) => field.handleChange(e.target.value)}
                        type="text"
                        placeholder="落下速度"
                      />
                      <InputGroupAddon align="inline-start">
                        <ChevronsDown className="text-muted-foreground" />
                      </InputGroupAddon>
                      <InputGroupAddon align="inline-end">
                        <InputGroupText className="text-muted-foreground">m/s</InputGroupText>
                      </InputGroupAddon>
                    </InputGroup>
                    {field.state.meta.isTouched && field.state.meta.errors.length ? (
                      <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                    ) : null}
                  </Field>
                )}
              />
            </FieldGroup>

            {/* バースト高度 */}
            <form.Field
              name="burstAltitude"
              validators={{
                onChange: ({ value }) => validatePositiveNumber(value, "バースト高度"),
              }}
              children={(field) => (
                <Field>
                  <FieldLabel className="text-xs" htmlFor={field.name}>バースト高度</FieldLabel>
                  <InputGroup>
                    <InputGroupInput
                      id={field.name}
                      name={field.name}
                      value={field.state.value}
                      onBlur={field.handleBlur}
                      onChange={(e) => field.handleChange(e.target.value)}
                      type="text"
                      placeholder="高度を入力"
                    />
                    <InputGroupAddon align="inline-start">
                      <Loader className="text-muted-foreground" />
                    </InputGroupAddon>
                    <InputGroupAddon align="inline-end">
                      <InputGroupText className="text-muted-foreground">m</InputGroupText>
                    </InputGroupAddon>
                  </InputGroup>
                  {field.state.meta.isTouched && field.state.meta.errors.length ? (
                    <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                  ) : null}
                </Field>
              )}
            />

            {/* 気象データソース */}
            <form.Subscribe
              selector={(state) => state.values.weatherSource}
              children={(weatherSource) => (
                <Field>
                  <FieldLabel className="text-xs flex items-center gap-1.5">
                    <Layers className="size-3.5" />
                    気象データソース
                  </FieldLabel>
                  <Select
                    value={weatherSource}
                    onValueChange={(val) => form.setFieldValue("weatherSource", val as WeatherSource)}
                  >
                    <SelectTrigger className="w-full mt-1.5">
                      <SelectValue placeholder="データソースを選択" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="gfs">GFS (0.25°)</SelectItem>
                      <SelectItem value="gefs">GEFS (アンサンブル)</SelectItem>
                    </SelectContent>
                  </Select>
                </Field>
              )}
            />

            {/* アンサンブル / モンテカルロ設定 */}
            <form.Subscribe
              selector={(state) => state.values.weatherSource}
              children={(weatherSource) => (
                <>
                  {weatherSource === "gefs" && (
                    <form.Field
                      name="gefsNumMembers"
                      validators={{
                        onChange: ({ value }) => {
                          if (!value) return "メンバー数を入力してください"
                          const num = Number(value)
                          if (isNaN(num) || !Number.isInteger(num)) return "整数で入力してください"
                          if (num < 1 || num > 31) return "1〜31 の範囲で入力してください"
                          return undefined
                        },
                      }}
                      children={(field) => (
                        <Field>
                          <FieldLabel className="text-xs" htmlFor={field.name}>アンサンブルメンバー数</FieldLabel>
                          <InputGroup>
                            <InputGroupInput
                              id={field.name}
                              name={field.name}
                              value={field.state.value}
                              onBlur={field.handleBlur}
                              onChange={(e) => field.handleChange(e.target.value)}
                              type="text"
                              placeholder="1-31"
                            />
                            <InputGroupAddon align="inline-start">
                              <Layers className="text-muted-foreground" />
                            </InputGroupAddon>
                            <InputGroupAddon align="inline-end">
                              <InputGroupText className="text-muted-foreground">メンバー</InputGroupText>
                            </InputGroupAddon>
                          </InputGroup>
                          {field.state.meta.isTouched && field.state.meta.errors.length ? (
                            <p className="text-[11px] text-red-500 mt-1">{field.state.meta.errors.join(", ")}</p>
                          ) : null}
                        </Field>
                      )}
                    />
                  )}

                  {weatherSource === "gfs" ? (
                    <form.Subscribe
                      selector={(state) => state.values.monteCarloEnabled}
                      children={(monteCarloEnabled) => (
                        <>
                          <Field>
                            <div className="flex items-center justify-between">
                              <FieldLabel className="text-xs flex items-center gap-1.5">
                                モンテカルロ法
                              </FieldLabel>
                              <button
                                type="button"
                                onClick={() => form.setFieldValue("monteCarloEnabled", !monteCarloEnabled)}
                                className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${
                                  monteCarloEnabled ? "bg-primary" : "bg-input"
                                }`}
                              >
                                <span
                                  className={`pointer-events-none block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform ${
                                    monteCarloEnabled ? "translate-x-4" : "translate-x-0"
                                  }`}
                                />
                              </button>
                            </div>
                          </Field>

                          {monteCarloEnabled && scatterFields}
                        </>
                      )}
                    />
                  ) : (
                    scatterFields
                  )}
                </>
              )}
            />
          </SidebarMenuItem>
        </SidebarContent>

        <SidebarFooter>
          <form.Subscribe
            selector={(state) => [state.canSubmit, state.isSubmitting]}
            children={([canSubmit, isSubmitting]) => (
              <Button type="submit" disabled={!canSubmit || isSubmitting || !!progress} className="w-full">
                {isSubmitting || progress ? (
                  <Loader className="animate-spin mr-2 h-4 w-4" />
                ) : (
                  <Play className="mr-2 h-4 w-4" />
                )}
                {progress ? formatProgressText(progress) : "予測"}
              </Button>
            )}
          />
        </SidebarFooter>
      </form>
    </Sidebar>
  )
}