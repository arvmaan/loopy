// V2 Types — matches engine.rs EngineState + web_v2.rs API

export type Phase =
  | 'initializing'
  | 'scanning'
  | 'planning'
  | 'awaiting_plan_review'
  | 'setting_up_workspaces'
  | 'running_tracks'
  | 'awaiting_code_review'
  | 'awaiting_test_plan'
  | 'test_flying'
  | 'awaiting_test_review'
  | 'landing'
  | 'complete'
  | 'failed';

export type StageId = 'idea' | 'scan' | 'plan' | 'orbital_lanes' | 'test_flight' | 'land';
export type StageStatus = 'pending' | 'running' | 'complete' | 'failed';
export type TrackStatus = 'pending' | 'running' | 'complete' | 'failed' | 'skipped';

export interface StageState {
  id: StageId;
  status: StageStatus;
  loop_id: string | null;
  loop_pid: number | null;
  started_at: string | null;
  completed_at: string | null;
  error: string | null;
}

export interface TrackState {
  id: string;
  name: string;
  status: TrackStatus;
  loop_id: string | null;
  loop_pid: number | null;
  current_sub_stage: string | null;
}

export interface EngineState {
  phase: Phase;
  idea: string;
  project_name: string;
  stages: StageState[];
  tracks: TrackState[] | null;
  plan_feedback_history: string[];
  code_feedback_history: string[];
  plan_iteration: number;
  code_iteration: number;
  template_id?: string;
  custom_pipeline?: { stages: { id: string }[] } | null;
  created_at: string;
  updated_at: string;
}

export interface ProjectSummary {
  name: string;
  phase: Phase;
  idea: string;
  created_at: string;
  updated_at: string;
}

// WebSocket messages
export interface LinearStageView {
  id: string;
  label: string;
  description?: string;
  optional: boolean;
  checkpoint: boolean;
}

export interface LinearProgress {
  template_id: string;
  stages: LinearStageView[];
  current: string | null;
  completed: string[];
  paused: string | null;
  done: boolean;
}

export type WsServerMsg =
  | { type: 'state'; data: EngineState }
  | { type: 'phase_change'; phase: Phase }
  | { type: 'log'; line: string; level: string }
  | { type: 'track_progress'; track: string; tasks_done: number; tasks_total: number; current: string }
  | ({ type: 'linear_progress' } & LinearProgress);

export type WsClientMsg =
  | { type: 'approve' }
  | { type: 'reject'; feedback: string }
  | { type: 'abort' };

// Diff types (same as v1)
export type DiffLineKind = 'Added' | 'Removed' | 'Context';

export interface DiffLine {
  kind: DiffLineKind;
  content: string;
}

export interface Hunk {
  header: string;
  lines: DiffLine[];
}

export interface DiffFile {
  path: string;
  hunks: Hunk[];
}

// Grouped review diff (Flight Check) — track → package → files
export interface ReviewPackage {
  package: string;
  files: DiffFile[];
}

export interface ReviewGroup {
  track: string;
  packages: ReviewPackage[];
  file_count: number;
}

export interface ReviewDiff {
  groups: ReviewGroup[];
}

// An inline review comment placed on a specific diff line.
export interface ReviewComment {
  track: string;
  package: string;
  file: string;
  line: number; // index into the file's flattened diff lines
  text: string;
}
