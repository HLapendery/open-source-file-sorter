import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AutomationStatus,
  Config,
  DesktopSnapshot,
  HistoryEntry,
  PlannedMove,
  RunReport,
} from "./types";

export const api = {
  snapshot: () => invoke<DesktopSnapshot>("load_snapshot"),
  saveConfig: (config: Config) =>
    invoke<Config>("save_configuration", { config }),
  testFilename: (config: Config, filename: string) =>
    invoke<PlannedMove | null>("test_filename", { config, filename }),
  preview: (directoryId: string) =>
    invoke<RunReport>("preview_directory", { directoryId }),
  execute: (directoryId: string) =>
    invoke<RunReport>("execute_directory", { directoryId }),
  history: (limit = 100) =>
    invoke<HistoryEntry[]>("load_history", { limit }),
  startAutomation: (directoryId: string) =>
    invoke<AutomationStatus[]>("start_automation", { directoryId }),
  stopAutomation: (directoryId: string) =>
    invoke<AutomationStatus[]>("stop_automation", { directoryId }),
  automationStatus: () =>
    invoke<AutomationStatus[]>("automation_status"),
};

export async function chooseDirectory(): Promise<string | null> {
  return open({ directory: true, multiple: false });
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Une erreur inattendue est survenue";
}
