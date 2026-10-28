<script setup lang="ts">
import { computed, ref } from "vue";
import { formatDate, stateLabel, t } from "../i18n";
import type { HistoryEntry } from "../types";

const props = defineProps<{ history: HistoryEntry[]; loading: boolean }>();
const emit = defineEmits<{ refresh: [] }>();
const filter = ref("all");

const visible = computed(() => {
  if (filter.value === "all") return props.history;
  if (filter.value === "attention") {
    return props.history.filter((entry) => entry.state === "FAILED" || entry.state === "RECOVERY_REQUIRED");
  }
  return props.history.filter((entry) => entry.state === filter.value);
});
</script>

<template>
  <section class="view-stack">
    <header class="view-heading">
      <div>
        <p class="eyebrow">{{ t("history.eyebrow") }}</p>
        <h1>{{ t("history.title") }}</h1>
        <p class="subtitle">{{ t("history.subtitle") }}</p>
      </div>
      <button class="secondary" :disabled="loading" @click="emit('refresh')">{{ loading ? t("history.refreshing") : t("history.refresh") }}</button>
    </header>

    <article class="panel table-panel">
      <div class="filter-bar">
        <button :class="{ active: filter === 'all' }" @click="filter = 'all'">{{ t("history.all") }}</button>
        <button :class="{ active: filter === 'COMPLETED' }" @click="filter = 'COMPLETED'">{{ t("history.completed") }}</button>
        <button :class="{ active: filter === 'SKIPPED' }" @click="filter = 'SKIPPED'">{{ t("history.skipped") }}</button>
        <button :class="{ active: filter === 'attention' }" @click="filter = 'attention'">{{ t("history.attention") }}</button>
        <span class="grow" />
        <small>{{ t("history.operationCount", { count: visible.length }) }}</small>
      </div>
      <div v-if="visible.length" class="table-scroll">
        <table>
          <thead><tr><th>{{ t("history.date") }}</th><th>{{ t("history.file") }}</th><th>{{ t("history.destination") }}</th><th>{{ t("history.rule") }}</th><th>{{ t("history.status") }}</th></tr></thead>
          <tbody>
            <tr v-for="entry in visible" :key="entry.id">
              <td class="nowrap">{{ formatDate(entry.updated_at_unix_ms) }}</td>
              <td><span class="file-name">{{ entry.source.split(/[\\/]/).pop() }}</span><small>{{ entry.source }}</small></td>
              <td><span class="file-name">{{ entry.destination.split(/[\\/]/).pop() }}</span><small>{{ entry.destination }}</small></td>
              <td><span class="pill">{{ entry.rule_id }}</span></td>
              <td>
                <span class="pill" :class="entry.state.toLowerCase()">{{ stateLabel(entry.state) }}</span>
                <small v-if="entry.error" class="error-text">{{ entry.error }}</small>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="empty-state roomy">
        <div class="empty-icon">H</div>
        <h2>{{ t("history.emptyTitle") }}</h2>
        <p>{{ t("history.emptyHint") }}</p>
      </div>
    </article>
  </section>
</template>
