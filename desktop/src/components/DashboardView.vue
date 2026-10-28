<script setup lang="ts">
import { computed } from "vue";
import type { AutomationStatus, Config, HistoryEntry, ViewName } from "../types";

const props = defineProps<{
  config: Config;
  history: HistoryEntry[];
  automation: AutomationStatus[];
}>();

const emit = defineEmits<{ navigate: [view: ViewName] }>();
const activeWorkers = computed(
  () => props.automation.filter((entry) => entry.state === "running" || entry.state === "stopping").length,
);
const failures = computed(
  () => props.history.filter((entry) => entry.state === "FAILED" || entry.state === "RECOVERY_REQUIRED").length,
);
</script>

<template>
  <section class="view-stack">
    <header class="view-heading">
      <div>
        <p class="eyebrow">Vue d’ensemble</p>
        <h1>Vos fichiers, rangés avec prudence.</h1>
        <p class="subtitle">Configuration, automatisation et historique dans une seule interface.</p>
      </div>
      <button class="primary" @click="emit('navigate', 'preview')">Prévisualiser un tri</button>
    </header>

    <div class="metric-grid">
      <article class="metric-card accent-card">
        <span>Répertoires</span>
        <strong>{{ config.directories.length }}</strong>
        <small>{{ config.directories.filter((item) => item.enabled).length }} actifs</small>
      </article>
      <article class="metric-card">
        <span>Règles</span>
        <strong>{{ config.rules.length }}</strong>
        <small>{{ config.rules.filter((item) => item.enabled).length }} activées</small>
      </article>
      <article class="metric-card">
        <span>Automatisations</span>
        <strong>{{ activeWorkers }}</strong>
        <small>workers en cours</small>
      </article>
      <article class="metric-card" :class="{ warning: failures > 0 }">
        <span>À vérifier</span>
        <strong>{{ failures }}</strong>
        <small>échecs ou récupérations</small>
      </article>
    </div>

    <div class="dashboard-grid">
      <article class="panel">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">Répertoires</p>
            <h2>Flux configurés</h2>
          </div>
          <button class="text-button" @click="emit('navigate', 'directories')">Gérer</button>
        </div>
        <div v-if="config.directories.length" class="item-list">
          <div v-for="directory in config.directories" :key="directory.id" class="list-row">
            <div class="status-dot" :class="directory.enabled ? 'online' : 'muted'" />
            <div class="grow min-width-zero">
              <strong>{{ directory.id }}</strong>
              <p class="truncate">{{ directory.path }}</p>
            </div>
            <span class="pill">{{ directory.mode }}</span>
          </div>
        </div>
        <div v-else class="empty-state compact">
          <strong>Aucun répertoire</strong>
          <p>Ajoutez une source pour commencer.</p>
        </div>
      </article>

      <article class="panel">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">Activité</p>
            <h2>Opérations récentes</h2>
          </div>
          <button class="text-button" @click="emit('navigate', 'history')">Tout voir</button>
        </div>
        <div v-if="history.length" class="item-list">
          <div v-for="entry in history.slice(0, 6)" :key="entry.id" class="list-row">
            <span class="state-mark" :class="entry.state.toLowerCase()">{{ entry.state === "COMPLETED" ? "✓" : "!" }}</span>
            <div class="grow min-width-zero">
              <strong class="truncate">{{ entry.source.split(/[\\/]/).pop() }}</strong>
              <p>{{ entry.rule_id }} · {{ new Date(entry.updated_at_unix_ms).toLocaleString() }}</p>
            </div>
            <span class="pill" :class="entry.state.toLowerCase()">{{ entry.state }}</span>
          </div>
        </div>
        <div v-else class="empty-state compact">
          <strong>Aucune opération</strong>
          <p>L’historique apparaîtra après le premier tri.</p>
        </div>
      </article>
    </div>
  </section>
</template>
