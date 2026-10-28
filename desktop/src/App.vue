<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api, errorMessage } from "./api";
import DashboardView from "./components/DashboardView.vue";
import DirectoriesView from "./components/DirectoriesView.vue";
import HistoryView from "./components/HistoryView.vue";
import PreviewView from "./components/PreviewView.vue";
import RulesView from "./components/RulesView.vue";
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

const navItems: Array<{ id: ViewName; label: string; short: string }> = [
  { id: "dashboard", label: "Tableau de bord", short: "TB" },
  { id: "directories", label: "Répertoires", short: "D" },
  { id: "rules", label: "Règles", short: "R" },
  { id: "preview", label: "Prévisualisation", short: "P" },
  { id: "history", label: "Historique", short: "H" },
];

const activeWorkers = computed(() =>
  automation.value.filter((entry) => entry.state === "running" || entry.state === "stopping"),
);

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
    showNotice("Configuration validée et enregistrée");
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
    showNotice(`Automatisation démarrée pour ${directoryId}`);
  } catch (caught) {
    showError(errorMessage(caught));
  }
}

async function stopAutomation(directoryId: string) {
  try {
    automation.value = await api.stopAutomation(directoryId);
    showNotice(`Arrêt demandé pour ${directoryId}`);
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
      <div class="brand">
        <div class="brand-mark"><span /><span /><span /></div>
        <div><strong>File Sorter</strong><small>open source</small></div>
      </div>

      <nav aria-label="Navigation principale">
        <button
          v-for="item in navItems"
          :key="item.id"
          :class="{ active: activeView === item.id }"
          @click="activeView = item.id"
        >
          <span class="nav-icon">{{ item.short }}</span>
          <span>{{ item.label }}</span>
          <span v-if="item.id === 'directories' && activeWorkers.length" class="nav-count">{{ activeWorkers.length }}</span>
        </button>
      </nav>

      <div class="sidebar-status">
        <div class="engine-status"><span class="status-dot online" /> Moteur Rust prêt</div>
        <p class="truncate" :title="configPath">{{ configPath || "Configuration locale" }}</p>
        <p class="truncate" :title="databasePath">{{ databasePath || "Journal SQLite" }}</p>
      </div>
    </aside>

    <main class="content-area">
      <div v-if="loading" class="loading-screen"><div class="loader" /><strong>Chargement du moteur…</strong></div>
      <div v-else-if="!config" class="loading-screen error-screen">
        <strong>Impossible d’initialiser l’application</strong><p>{{ error }}</p>
        <button class="primary" @click="loadSnapshot">Réessayer</button>
      </div>
      <template v-else>
        <div class="topbar">
          <div class="save-state"><span class="status-dot" :class="dirty ? 'warning-dot' : 'online'" />{{ dirty ? "Modifications non enregistrées" : "Configuration à jour" }}</div>
          <button class="primary save-button" :disabled="!dirty || saving" @click="save">{{ saving ? "Validation…" : "Enregistrer" }}</button>
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

    <div v-if="error && config" class="toast error-toast"><strong>Erreur</strong><span>{{ error }}</span><button @click="error = ''">×</button></div>
    <div v-if="notice" class="toast notice-toast"><strong>OK</strong><span>{{ notice }}</span><button @click="notice = ''">×</button></div>
  </div>
</template>
