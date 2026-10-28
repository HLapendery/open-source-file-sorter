<script setup lang="ts">
import { computed, ref } from "vue";
import { api, errorMessage } from "../api";
import { stateLabel, t } from "../i18n";
import type { Config, RunReport } from "../types";

const props = defineProps<{ config: Config; dirty: boolean }>();
const emit = defineEmits<{ executed: []; error: [message: string] }>();

const selectedId = ref(props.config.directories[0]?.id || "");
const report = ref<RunReport | null>(null);
const busy = ref<"preview" | "execute" | null>(null);
const selectedDirectory = computed(() =>
  props.config.directories.find((entry) => entry.id === selectedId.value),
);

async function preview() {
  if (!selectedId.value) return;
  busy.value = "preview";
  report.value = null;
  try {
    report.value = await api.preview(selectedId.value);
  } catch (error) {
    emit("error", errorMessage(error));
  } finally {
    busy.value = null;
  }
}

async function execute() {
  if (!selectedId.value) return;
  const count = report.value?.items.length || 0;
  if (!window.confirm(t("preview.confirmExecute", { count }))) return;
  busy.value = "execute";
  try {
    report.value = await api.execute(selectedId.value);
    emit("executed");
  } catch (error) {
    emit("error", errorMessage(error));
  } finally {
    busy.value = null;
  }
}
</script>

<template>
  <section class="view-stack">
    <header class="view-heading">
      <div>
        <p class="eyebrow">{{ t("preview.eyebrow") }}</p>
        <h1>{{ t("preview.title") }}</h1>
        <p class="subtitle">{{ t("preview.subtitle") }}</p>
      </div>
    </header>

    <article class="panel preview-controls">
      <label class="grow">
        <span>{{ t("preview.configuredDirectory") }}</span>
        <select v-model="selectedId" @change="report = null">
          <option disabled value="">{{ t("preview.selectSource") }}</option>
          <option v-for="directory in config.directories" :key="directory.id" :value="directory.id">
            {{ directory.id }} — {{ directory.path }}
          </option>
        </select>
      </label>
      <div class="button-row align-end">
        <button class="secondary" :disabled="!selectedId || dirty || busy !== null" @click="preview">
          {{ busy === "preview" ? t("preview.analyzing") : t("preview.generate") }}
        </button>
        <button class="primary" :disabled="!report || report.items.length === 0 || report.dry_run === false || busy !== null" @click="execute">
          {{ busy === "execute" ? t("preview.executing") : t("preview.execute") }}
        </button>
      </div>
      <p v-if="dirty" class="inline-warning full-width">{{ t("preview.saveFirst") }}</p>
      <p v-else-if="selectedDirectory" class="path-note full-width">
        {{ selectedDirectory.recursive ? t("preview.recursive") : t("preview.firstLevel") }} ·
        {{ t("preview.ruleCount", { count: selectedDirectory.rules.length }) }} · {{ t("preview.revalidate") }}
      </p>
    </article>

    <template v-if="report">
      <div class="metric-grid compact-metrics">
        <article class="metric-card"><span>{{ t("preview.scanned") }}</span><strong>{{ report.scanned }}</strong></article>
        <article class="metric-card"><span>{{ t("preview.matches") }}</span><strong>{{ report.matched }}</strong></article>
        <article class="metric-card"><span>{{ report.dry_run ? t("preview.planned") : t("preview.moved") }}</span><strong>{{ report.dry_run ? report.items.length : report.moved }}</strong></article>
        <article class="metric-card" :class="{ warning: report.failed > 0 }"><span>{{ t("preview.failures") }}</span><strong>{{ report.failed + report.scan_errors.length }}</strong></article>
      </div>

      <article class="panel table-panel">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">{{ report.dry_run ? t("preview.proposedPlan") : t("preview.result") }}</p>
            <h2>{{ t("preview.actions", { count: report.items.length }) }}</h2>
          </div>
          <span class="pill" :class="report.dry_run ? '' : 'completed'">{{ report.dry_run ? "DRY-RUN" : t("preview.executed") }}</span>
        </div>
        <div v-if="report.items.length" class="table-scroll">
          <table>
            <thead><tr><th>{{ t("preview.source") }}</th><th>{{ t("preview.destination") }}</th><th>{{ t("preview.rule") }}</th><th>{{ t("preview.status") }}</th></tr></thead>
            <tbody>
              <tr v-for="(item, index) in report.items" :key="`${item.source}-${index}`">
                <td><span class="file-name">{{ item.source.split(/[\\/]/).pop() }}</span><small>{{ item.source }}</small></td>
                <td><span class="file-name">{{ item.destination.split(/[\\/]/).pop() }}</span><small>{{ item.destination }}</small></td>
                <td><span class="pill">{{ item.rule_id }}</span></td>
                <td><span class="pill" :class="item.status">{{ stateLabel(item.status) }}</span><small v-if="item.error" class="error-text">{{ item.error }}</small></td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="empty-state compact">
          <strong>{{ t("preview.noMatches") }}</strong>
          <p>{{ t("preview.noMatchesHint") }}</p>
        </div>
        <div v-if="report.scan_errors.length" class="error-box">
          <strong>{{ t("preview.scanErrors") }}</strong>
          <p v-for="error in report.scan_errors" :key="error">{{ error }}</p>
        </div>
      </article>
    </template>

    <div v-else class="empty-state panel roomy">
      <div class="empty-icon">P</div>
      <h2>{{ t("preview.emptyTitle") }}</h2>
      <p>{{ t("preview.emptyHint") }}</p>
    </div>
  </section>
</template>
