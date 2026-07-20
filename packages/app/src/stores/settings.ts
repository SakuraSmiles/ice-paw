// IcePaw 设置状态管理 Store
import { ref } from "vue";
import { defineStore } from "pinia";
import { bridge } from "../api/bridge";
import type { UserPreferences } from "../types";

const SAVE_DEBOUNCE_MS = 500;

export const useSettingsStore = defineStore("settings", () => {
  const prefs = ref<UserPreferences>({});
  const loading = ref<boolean>(false);
  const error = ref<string | null>(null);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  async function load(): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      const data = await bridge.preferences.get();
      prefs.value = data;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
    } finally {
      loading.value = false;
    }
  }

  function update(key: keyof UserPreferences, value: unknown): void {
    (prefs.value as Record<string, unknown>)[key] = value;
    if (debounceTimer !== null) {
      clearTimeout(debounceTimer);
    }
    debounceTimer = setTimeout(async () => {
      debounceTimer = null;
      try {
        const serializedValue =
          value === null || value === undefined
            ? JSON.stringify(null)
            : JSON.stringify(value);
        await bridge.preferences.set(key, serializedValue);
      } catch (err) {
        error.value = err instanceof Error ? err.message : String(err);
      }
    }, SAVE_DEBOUNCE_MS);
  }

  function clearError(): void {
    error.value = null;
  }

  return { prefs, loading, error, load, update, clearError };
});
