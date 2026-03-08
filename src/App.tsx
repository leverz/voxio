import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isRegistered,
  register,
  unregister,
} from "@tauri-apps/plugin-global-shortcut";
import { useEffect, useState } from "react";
import type {
  AppStateSnapshot,
  PermissionStatus,
  Settings,
  StateChangedEvent,
} from "./types";

const DEFAULT_STATE: AppStateSnapshot = {
  state: "idle",
  sessionId: null,
  lastTranscript: null,
  lastError: null,
};

const DEFAULT_SETTINGS: Settings = {
  hotkey: "Option+Space",
  language: "auto",
  autoPunctuation: true,
  silenceTimeoutMs: 1200,
  injectionMode: "auto",
  model: "base",
  launchAtLogin: false,
};

const DEFAULT_PERMISSIONS: PermissionStatus = {
  microphone: false,
  accessibility: false,
  inputMonitoring: false,
};

const STATE_LABELS: Record<AppStateSnapshot["state"], string> = {
  idle: "Idle",
  listening: "Listening...",
  processing: "Processing speech...",
  error: "Needs attention",
};

export function App() {
  const [appState, setAppState] = useState<AppStateSnapshot>(DEFAULT_STATE);
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [permissions, setPermissions] =
    useState<PermissionStatus>(DEFAULT_PERMISSIONS);
  const [isSaving, setIsSaving] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    let unlisten: UnlistenFn | null = null;

    async function bootstrap() {
      const [stateSnapshot, appSettings, permissionStatus] = await Promise.all([
        invoke<AppStateSnapshot>("get_app_state"),
        invoke<Settings>("get_settings"),
        invoke<PermissionStatus>("request_permissions"),
      ]);

      if (!mounted) {
        return;
      }

      setAppState(stateSnapshot);
      setSettings(appSettings);
      setPermissions(permissionStatus);

      unlisten = await listen<StateChangedEvent>(
        "voxio://state-changed",
        (event) => {
          setAppState(event.payload.snapshot);
        },
      );
    }

    bootstrap().catch((error: unknown) => {
      setBanner(`Failed to initialize app shell: ${String(error)}`);
    });

    return () => {
      mounted = false;
      if (unlisten) {
        void unlisten();
      }
    };
  }, []);

  useEffect(() => {
    let activeShortcut: string | null = null;

    async function syncShortcut() {
      if (!(window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
        return;
      }

      const accelerator = settings.hotkey.trim();
      if (!accelerator) {
        return;
      }

      try {
        if (activeShortcut && activeShortcut !== accelerator) {
          const registered = await isRegistered(activeShortcut);
          if (registered) {
            await unregister(activeShortcut);
          }
        }

        const alreadyRegistered = await isRegistered(accelerator);
        if (!alreadyRegistered) {
          await register(accelerator, async (event) => {
            if (event.state !== "Pressed") {
              return;
            }

            const snapshot = await invoke<AppStateSnapshot>("toggle_dictation");
            setAppState(snapshot);
          });
        }

        activeShortcut = accelerator;
      } catch (error) {
        setBanner(`Failed to register global shortcut: ${String(error)}`);
      }
    }

    void syncShortcut();

    return () => {
      if (!activeShortcut) {
        return;
      }

      void unregister(activeShortcut);
    };
  }, [settings.hotkey]);

  async function persistSettings(nextSettings: Settings) {
    setIsSaving(true);
    setBanner(null);

    try {
      const stored = await invoke<Settings>("update_settings", {
        payload: nextSettings,
      });

      setSettings(stored);
      setBanner(null);
    } catch (error) {
      setBanner(`Failed to save settings: ${String(error)}`);
    } finally {
      setIsSaving(false);
    }
  }

  async function handlePrimaryAction() {
    try {
      if (appState.state === "idle" || appState.state === "error") {
        const snapshot = await invoke<AppStateSnapshot>("start_dictation");
        setAppState(snapshot);
        return;
      }

      if (appState.state === "listening") {
        const snapshot = await invoke<AppStateSnapshot>("stop_dictation");
        setAppState(snapshot);
        return;
      }

      const snapshot = await invoke<AppStateSnapshot>("cancel_dictation");
      setAppState(snapshot);
    } catch (error) {
      setBanner(String(error));
    }
  }

  async function handleCancel() {
    try {
      const snapshot = await invoke<AppStateSnapshot>("cancel_dictation");
      setAppState(snapshot);
    } catch (error) {
      setBanner(String(error));
    }
  }

  return (
    <main className="shell">
      <section className="hero">
        <div className="hero__badge">Open-source voice typing for desktop</div>
        <div className="hero__status">
          <div
            className={`status-dot status-dot--${appState.state}`}
            aria-hidden="true"
          />
          <span>{STATE_LABELS[appState.state]}</span>
        </div>
        <h1>Voxio</h1>
        <p className="hero__copy">
          Press a global shortcut, speak naturally, then insert text back into
          the active app without changing focus.
        </p>
        <div className="hero__actions">
          <button className="button button--primary" onClick={handlePrimaryAction}>
            {appState.state === "listening" ? "Stop dictation" : "Start dictation"}
          </button>
          <button
            className="button button--ghost"
            onClick={handleCancel}
            disabled={appState.state === "idle"}
          >
            Cancel
          </button>
        </div>
        <div className="hero__hint">
          Current shortcut <strong>{settings.hotkey}</strong>
        </div>
        <div className="hero__hint hero__hint--warning">
          Current build pastes a placeholder transcript when dictation stops.
        </div>
      </section>

      <section className="grid">
        <article className="panel panel--state">
          <div className="panel__eyebrow">Session</div>
          <h2>Dictation state</h2>
          <dl className="facts">
            <div>
              <dt>State</dt>
              <dd>{appState.state}</dd>
            </div>
            <div>
              <dt>Session ID</dt>
              <dd>{appState.sessionId ?? "None"}</dd>
            </div>
            <div>
              <dt>Last transcript</dt>
              <dd>{appState.lastTranscript ?? "No transcript yet"}</dd>
            </div>
            <div>
              <dt>Last error</dt>
              <dd>{appState.lastError ?? "No errors"}</dd>
            </div>
          </dl>
        </article>

        <article className="panel">
          <div className="panel__eyebrow">Permissions</div>
          <h2>System readiness</h2>
          <ul className="checklist">
            <li data-ready={permissions.microphone}>Microphone access</li>
            <li data-ready={permissions.accessibility}>Accessibility access</li>
            <li data-ready={permissions.inputMonitoring}>Input monitoring</li>
          </ul>
          <p className="panel__note">
            The current Rust shell reports placeholders until native permission
            checks are implemented.
          </p>
        </article>

        <article className="panel panel--settings">
          <div className="panel__eyebrow">Settings</div>
          <h2>Input preferences</h2>
          <div className="form-grid">
            <label>
              Hotkey
              <input
                value={settings.hotkey}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    hotkey: event.target.value,
                  }))
                }
              />
            </label>
            <label>
              Language
              <select
                value={settings.language}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    language: event.target.value,
                  }))
                }
              >
                <option value="auto">Auto detect</option>
                <option value="en">English</option>
                <option value="zh">Chinese</option>
              </select>
            </label>
            <label>
              Silence timeout (ms)
              <input
                type="number"
                min={500}
                max={5000}
                step={100}
                value={settings.silenceTimeoutMs}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    silenceTimeoutMs: Number(event.target.value),
                  }))
                }
              />
            </label>
            <label>
              Injection mode
              <select
                value={settings.injectionMode}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    injectionMode: event.target.value as Settings["injectionMode"],
                  }))
                }
              >
                <option value="auto">Auto fallback</option>
                <option value="accessibility">Accessibility</option>
                <option value="clipboard">Clipboard paste</option>
              </select>
            </label>
            <label>
              Model
              <select
                value={settings.model}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    model: event.target.value as Settings["model"],
                  }))
                }
              >
                <option value="base">Base</option>
                <option value="small">Small</option>
              </select>
            </label>
            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.autoPunctuation}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    autoPunctuation: event.target.checked,
                  }))
                }
              />
              Auto punctuation
            </label>
            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.launchAtLogin}
                onChange={(event) =>
                  setSettings((current) => ({
                    ...current,
                    launchAtLogin: event.target.checked,
                  }))
                }
              />
              Launch at login
            </label>
          </div>
          <div className="panel__actions">
            <button
              className="button button--primary"
              onClick={() => void persistSettings(settings)}
              disabled={isSaving}
            >
              {isSaving ? "Saving..." : "Save settings"}
            </button>
          </div>
        </article>
      </section>

      {banner ? <aside className="banner">{banner}</aside> : null}
    </main>
  );
}
