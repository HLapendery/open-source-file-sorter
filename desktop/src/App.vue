<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api, errorMessage } from "./api";
import DashboardView from "./components/DashboardView.vue";
import DirectoriesView from "./components/DirectoriesView.vue";
import HistoryView from "./components/HistoryView.vue";
import PreviewView from "./components/PreviewView.vue";
import RulesView from "./components/RulesView.vue";
import AppIcon from "./components/AppIcon.vue";
import { locale, setLocale, t, type Locale } from "./i18n";
import type { AutomationStatus, Config, HistoryEntry, ViewName } from "./types";

const activeView = ref<ViewName>("dashboard");
const config = ref<Config | null>(null);
const history = ref<HistoryEntry[]>([]);
const automation = ref<AutomationStatus[]>([]);
const configPath = ref("");
const databasePath = ref("");
const dirty = ref(false);
const loading = ref(true);
const saving = ref(false);
const historyLoading = ref(false);
const error = ref("");
const notice = ref("");
let statusTimer: number | undefined;
let messageTimer: number | undefined;

const navItems: Array<{ id: ViewName; label: Parameters<typeof t>[0]; icon: "overview" | "directories" | "rules" | "preview" | "history" }> = [
  { id: "dashboard", label: "nav.dashboard", icon: "overview" },
  { id: "directories", label: "nav.directories", icon: "directories" },
  { id: "rules", label: "nav.rules", icon: "rules" },
  { id: "preview", label: "nav.preview", icon: "preview" },
  { id: "history", label: "nav.history", icon: "history" },
];

const activeWorkers = computed(() =>
  automation.value.filter((entry) => entry.state === "running" || entry.state === "stopping"),
);
const activeTitle = computed(() => t(navItems.find((item) => item.id === activeView.value)?.label ?? "nav.dashboard"));

async function loadSnapshot() {
  loading.value = true;
  error.value = "";
  try {
    const snapshot = await api.snapshot();
    config.value = structuredClone(snapshot.config);
    history.value = snapshot.history;
    automation.value = snapshot.automation;
    configPath.value = snapshot.config_path;
    databasePath.value = snapshot.database_path;
    dirty.value = false;
  } catch (caught) {
    error.value = errorMessage(caught);
  } finally {
    loading.value = false;
  }
}

function markDirty() {
  dirty.value = true;
}

async function save() {
  if (!config.value) return;
  saving.value = true;
  error.value = "";
  try {
    config.value = structuredClone(await api.saveConfig(config.value));
    dirty.value = false;
    showNotice(t("notice.saved"));
  } catch (caught) {
    showError(errorMessage(caught));
  } finally {
    saving.value = false;
  }
}

async function refreshHistory() {
  historyLoading.value = true;
  try {
    history.value = await api.history();
  } catch (caught) {
    showError(errorMessage(caught));
  } finally {
    historyLoading.value = false;
  }
}

async function startAutomation(directoryId: string) {
  try {
    automation.value = await api.startAutomation(directoryId);
    showNotice(t("notice.started", { id: directoryId }));
  } catch (caught) {
    showError(errorMessage(caught));
  }
}

async function stopAutomation(directoryId: string) {
  try {
    automation.value = await api.stopAutomation(directoryId);
    showNotice(t("notice.stopping", { id: directoryId }));
  } catch (caught) {
    showError(errorMessage(caught));
  }
}

async function pollAutomation() {
  if (activeWorkers.value.length === 0) return;
  try {
    automation.value = await api.automationStatus();
    if (activeWorkers.value.length === 0) await refreshHistory();
  } catch (caught) {
    showError(errorMessage(caught));
  }
}

function showError(message: string) {
  error.value = message;
  window.clearTimeout(messageTimer);
  messageTimer = window.setTimeout(() => (error.value = ""), 8000);
}

function showNotice(message: string) {
  notice.value = message;
  window.clearTimeout(messageTimer);
  messageTimer = window.setTimeout(() => (notice.value = ""), 4000);
}

onMounted(async () => {
  await loadSnapshot();
  statusTimer = window.setInterval(pollAutomation, 2000);
});

onBeforeUnmount(() => {
  window.clearInterval(statusTimer);
  window.clearTimeout(messageTimer);
});
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand" :title="t('app.name')">
        <div class="brand-mark"><span /><span /><span /></div>
      </div>

      <nav :aria-label="t('app.name')">
        <button
          v-for="item in navItems"
          :key="item.id"
          :class="{ active: activeView === item.id }"
          :title="t(item.label)"
          :aria-label="t(item.label)"
          @click="activeView = item.id"
        >
          <span class="nav-icon"><AppIcon :name="item.icon" /></span>
          <span class="nav-label">{{ t(item.label) }}</span>
          <span v-if="item.id === 'directories' && activeWorkers.length" class="nav-count">{{ activeWorkers.length }}</span>
        </button>
      </nav>

      <div class="sidebar-status">
        <span class="status-dot online" />
        <span class="sr-only">{{ t("topbar.engineReady") }}</span>
      </div>
    </aside>

    <main class="content-area">
      <div v-if="loading" class="loading-screen"><div class="loader" /><strong>{{ t("app.loading") }}</strong></div>
      <div v-else-if="!config" class="loading-screen error-screen">
        <strong>{{ t("app.initError") }}</strong><p>{{ error }}</p>
        <button class="primary" @click="loadSnapshot">{{ t("common.retry") }}</button>
      </div>
      <template v-else>
        <div class="topbar">
          <div class="page-context">
            <strong>{{ activeTitle }}</strong>
            <span class="engine-status"><span class="status-dot online" />{{ t("topbar.engineReady") }}</span>
          </div>
          <div class="topbar-actions">
            <div class="save-state"><span class="status-dot" :class="dirty ? 'warning-dot' : 'online'" />{{ dirty ? t("topbar.unsaved") : t("topbar.upToDate") }}</div>
            <label class="language-picker">
              <AppIcon name="language" />
              <span class="sr-only">{{ t("topbar.language") }}</span>
              <select :value="locale" :aria-label="t('topbar.language')" @change="setLocale(($event.target as HTMLSelectElement).value as Locale)">
                <option value="en">EN</option>
                <option value="fr">FR</option>
              </select>
            </label>
            <button class="primary save-button" :disabled="!dirty || saving" @click="save">{{ saving ? t("topbar.saving") : t("topbar.save") }}</button>
          </div>
        </div>

        <div class="view-container">
          <DashboardView v-if="activeView === 'dashboard'" :config="config" :history="history" :automation="automation" @navigate="activeView = $event" />
          <DirectoriesView
            v-else-if="activeView === 'directories'"
            :config="config"
            :automation="automation"
            :dirty="dirty"
            @dirty="markDirty"
            @start="startAutomation"
            @stop="stopAutomation"
          />
          <RulesView v-else-if="activeView === 'rules'" :config="config" @dirty="markDirty" />
          <PreviewView v-else-if="activeView === 'preview'" :config="config" :dirty="dirty" @executed="refreshHistory" @error="showError" />
          <HistoryView v-else :history="history" :loading="historyLoading" @refresh="refreshHistory" />
        </div>
      </template>
    </main>

    <div v-if="error && config" class="toast error-toast"><strong>{{ t("common.error") }}</strong><span>{{ error }}</span><button @click="error = ''">×</button></div>
    <div v-if="notice" class="toast notice-toast"><strong>{{ t("common.success") }}</strong><span>{{ notice }}</span><button @click="notice = ''">×</button></div>
  </div>
</template>
