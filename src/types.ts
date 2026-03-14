export type DictationState = "idle" | "listening" | "processing" | "error";

export interface AppStateSnapshot {
  state: DictationState;
  sessionId: string | null;
  lastTranscript: string | null;
  lastError: string | null;
  lastProvider: string | null;
  lastLatencyMs: number | null;
  requestedBackend: string | null;
  actualBackend: string | null;
  detectedLanguage: string | null;
  fallbackUsed: boolean;
  fallbackReason: string | null;
}

export interface PermissionStatus {
  microphone: boolean;
  accessibility: boolean;
  inputMonitoring: boolean;
}

export interface RuntimeStatus {
  localReady: boolean;
  cloudReady: boolean;
  localBackend: string;
  effectiveProvider: string;
  whisper: BackendStatus;
  senseVoice: BackendStatus;
  cloud: BackendStatus;
  localStrategy: string;
}

export interface ProviderProbeResult {
  provider: string;
  ok: boolean;
  message: string;
}

export interface BackendStatus {
  name: string;
  ready: boolean;
  detail: string;
}

export type ProbeTarget =
  | "current"
  | "autoRoute"
  | "whisper"
  | "senseVoice"
  | "cloud";

export interface Settings {
  hotkey: string;
  language: string;
  localBackend: "auto" | "whisper" | "senseVoice";
  transcriptionHint: string;
  vocabularyTerms: string;
  autoPunctuation: boolean;
  silenceTimeoutMs: number;
  injectionMode: "auto" | "accessibility" | "clipboard";
  transcriptionProvider: "local" | "cloud" | "auto";
  cloudModel: "fast" | "accurate";
  model: "fast" | "balanced" | "small";
  launchAtLogin: boolean;
}

export interface StateChangedEvent {
  snapshot: AppStateSnapshot;
}
