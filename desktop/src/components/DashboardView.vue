<script setup lang="ts">
import { computed } from "vue";
import { formatDate, stateLabel, t } from "../i18n";
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
        <p class="eyebrow">{{ t("dashboard.eyebrow") }}</p>
        <h1>{{ t("dashboard.title") }}</h1>
        <p class="subtitle">{{ t("dashboard.subtitle") }}</p>
      </div>
      <button class="primary" @click="emit('navigate', 'preview')">{{ t("dashboard.preview") }}</button>
    </header>

    <div class="metric-grid">
      <article class="metric-card accent-card">
        <span>{{ t("dashboard.directories") }}</span>
        <strong>{{ config.directories.length }}</strong>
        <small>{{ t("dashboard.active", { count: config.directories.filter((item) => item.enabled).length }) }}</small>
      </article>
      <article class="metric-card">
        <span>{{ t("dashboard.rules") }}</span>
        <strong>{{ config.rules.length }}</strong>
        <small>{{ t("dashboard.enabled", { count: config.rules.filter((item) => item.enabled).length }) }}</small>
      </article>
      <article class="metric-card">
        <span>{{ t("dashboard.automations") }}</span>
        <strong>{{ activeWorkers }}</strong>
        <small>{{ t("dashboard.workers") }}</small>
      </article>
      <article class="metric-card" :class="{ warning: failures > 0 }">
        <span>{{ t("dashboard.attention") }}</span>
        <strong>{{ failures }}</strong>
        <small>{{ t("dashboard.failures") }}</small>
      </article>
    </div>

    <div class="dashboard-grid">
      <article class="panel">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">{{ t("dashboard.sourcesEyebrow") }}</p>
            <h2>{{ t("dashboard.configuredFlows") }}</h2>
          </div>
          <button class="text-button" @click="emit('navigate', 'directories')">{{ t("dashboard.manage") }}</button>
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
          <strong>{{ t("dashboard.noDirectories") }}</strong>
          <p>{{ t("dashboard.addSource") }}</p>
        </div>
      </article>

      <article class="panel">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">{{ t("dashboard.activity") }}</p>
            <h2>{{ t("dashboard.recentOperations") }}</h2>
          </div>
          <button class="text-button" @click="emit('navigate', 'history')">{{ t("dashboard.viewAll") }}</button>
        </div>
        <div v-if="history.length" class="item-list">
          <div v-for="entry in history.slice(0, 6)" :key="entry.id" class="list-row">
            <span class="state-mark" :class="entry.state.toLowerCase()">{{ entry.state === "COMPLETED" ? "✓" : "!" }}</span>
            <div class="grow min-width-zero">
              <strong class="truncate">{{ entry.source.split(/[\\/]/).pop() }}</strong>
              <p>{{ entry.rule_id }} · {{ formatDate(entry.updated_at_unix_ms) }}</p>
            </div>
            <span class="pill" :class="entry.state.toLowerCase()">{{ stateLabel(entry.state) }}</span>
          </div>
        </div>
        <div v-else class="empty-state compact">
          <strong>{{ t("dashboard.noOperations") }}</strong>
          <p>{{ t("dashboard.historyAfterFirst") }}</p>
        </div>
      </article>
    </div>
  </section>
</template>
