export interface TrajectoryPoint {
  lat: number
  lon: number
  alt: number
  time: number
}

export interface PredictionData {
  ascent_path: TrajectoryPoint[]
  descent_path: TrajectoryPoint[]
  stratosphere_duration_s: number
  max_altitude: number
  landing_lat: number
  landing_lon: number
  drift_km: number
  total_duration_s: number
}

export interface MonteCarloPoint {
  landing_lat: number
  landing_lon: number
  burst_altitude: number
  deviation_sigma: number | null
}

export interface MonteCarloTrajectory {
  ascent_path: TrajectoryPoint[]
  descent_path: TrajectoryPoint[]
}

export interface MonteCarloResult {
  points: MonteCarloPoint[]
  mean_landing_lat: number
  mean_landing_lon: number
  mean_ascent_path: TrajectoryPoint[]
  mean_descent_path: TrajectoryPoint[]
  trajectories: MonteCarloTrajectory[]
}