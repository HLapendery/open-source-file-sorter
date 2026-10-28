<script setup lang="ts">
import { computed, ref } from "vue";
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
        <p class="eyebrow">Journal SQLite</p>
        <h1>Historique</h1>
        <p class="subtitle">Chaque mutation réelle est persistée avec sa source, sa destination et son état.</p>
      </div>
      <button class="secondary" :disabled="loading" @click="emit('refresh')">{{ loading ? "Actualisation…" : "Actualiser" }}</button>
    </header>

    <article class="panel table-panel">
      <div class="filter-bar">
        <button :class="{ active: filter === 'all' }" @click="filter = 'all'">Toutes</button>
        <button :class="{ active: filter === 'COMPLETED' }" @click="filter = 'COMPLETED'">Terminées</button>
        <button :class="{ active: filter === 'SKIPPED' }" @click="filter = 'SKIPPED'">Ignorées</button>
        <button :class="{ active: filter === 'attention' }" @click="filter = 'attention'">À vérifier</button>
        <span class="grow" />
        <small>{{ visible.length }} opération(s)</small>
      </div>
      <div v-if="visible.length" class="table-scroll">
        <table>
          <thead><tr><th>Date</th><th>Fichier</th><th>Destination</th><th>Règle</th><th>État</th></tr></thead>
          <tbody>
            <tr v-for="entry in visible" :key="entry.id">
              <td class="nowrap">{{ new Date(entry.updated_at_unix_ms).toLocaleString() }}</td>
              <td><span class="file-name">{{ entry.source.split(/[\\/]/).pop() }}</span><small>{{ entry.source }}</small></td>
              <td><span class="file-name">{{ entry.destination.split(/[\\/]/).pop() }}</span><small>{{ entry.destination }}</small></td>
              <td><span class="pill">{{ entry.rule_id }}</span></td>
              <td>
                <span class="pill" :class="entry.state.toLowerCase()">{{ entry.state }}</span>
                <small v-if="entry.error" class="error-text">{{ entry.error }}</small>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="empty-state roomy">
        <div class="empty-icon">H</div>
        <h2>Aucune opération dans cette vue</h2>
        <p>Les dry-runs ne créent volontairement aucune entrée.</p>
      </div>
    </article>
  </section>
</template>
