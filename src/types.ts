export type DictationState = "idle" | "listening" | "processing" | "error";

export interface AppStateSnapshot {
  state: DictationState;
  sessionId: string | null;
  lastTranscript: string | null;
  lastError: string | null;
}

export interface PermissionStatus {
  microphone: boolean;
  accessibility: boolean;
  inputMonitoring: boolean;
}

export interface Settings {
  hotkey: string;
  language: string;
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
