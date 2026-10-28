<script setup lang="ts">
import { computed, ref } from "vue";
import { api, errorMessage } from "../api";
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
  if (!window.confirm(`Relancer l’analyse puis exécuter jusqu’à ${count} mouvement(s) avec le journal de sécurité ?`)) return;
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
        <p class="eyebrow">Contrôle avant action</p>
        <h1>Prévisualisation</h1>
        <p class="subtitle">Le dry-run utilise le même moteur Rust que l’exécution, sans mutation ni historique.</p>
      </div>
    </header>

    <article class="panel preview-controls">
      <label class="grow">
        <span>Répertoire configuré</span>
        <select v-model="selectedId" @change="report = null">
          <option disabled value="">Sélectionner une source</option>
          <option v-for="directory in config.directories" :key="directory.id" :value="directory.id">
            {{ directory.id }} — {{ directory.path }}
          </option>
        </select>
      </label>
      <div class="button-row align-end">
        <button class="secondary" :disabled="!selectedId || dirty || busy !== null" @click="preview">
          {{ busy === "preview" ? "Analyse…" : "Générer le plan" }}
        </button>
        <button class="primary" :disabled="!report || report.items.length === 0 || report.dry_run === false || busy !== null" @click="execute">
          {{ busy === "execute" ? "Exécution…" : "Analyser et exécuter" }}
        </button>
      </div>
      <p v-if="dirty" class="inline-warning full-width">Enregistrez la configuration avant l’analyse.</p>
      <p v-else-if="selectedDirectory" class="path-note full-width">
        {{ selectedDirectory.recursive ? "Analyse récursive" : "Premier niveau uniquement" }} ·
        {{ selectedDirectory.rules.length }} règle(s) · l’exécution revalide la source
      </p>
    </article>

    <template v-if="report">
      <div class="metric-grid compact-metrics">
        <article class="metric-card"><span>Analysés</span><strong>{{ report.scanned }}</strong></article>
        <article class="metric-card"><span>Correspondances</span><strong>{{ report.matched }}</strong></article>
        <article class="metric-card"><span>{{ report.dry_run ? "Planifiés" : "Déplacés" }}</span><strong>{{ report.dry_run ? report.items.length : report.moved }}</strong></article>
        <article class="metric-card" :class="{ warning: report.failed > 0 }"><span>Échecs</span><strong>{{ report.failed + report.scan_errors.length }}</strong></article>
      </div>

      <article class="panel table-panel">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">{{ report.dry_run ? "Plan proposé" : "Résultat" }}</p>
            <h2>{{ report.items.length }} action(s)</h2>
          </div>
          <span class="pill" :class="report.dry_run ? '' : 'completed'">{{ report.dry_run ? "DRY-RUN" : "EXÉCUTÉ" }}</span>
        </div>
        <div v-if="report.items.length" class="table-scroll">
          <table>
            <thead><tr><th>Source</th><th>Destination</th><th>Règle</th><th>État</th></tr></thead>
            <tbody>
              <tr v-for="(item, index) in report.items" :key="`${item.source}-${index}`">
                <td><span class="file-name">{{ item.source.split(/[\\/]/).pop() }}</span><small>{{ item.source }}</small></td>
                <td><span class="file-name">{{ item.destination.split(/[\\/]/).pop() }}</span><small>{{ item.destination }}</small></td>
                <td><span class="pill">{{ item.rule_id }}</span></td>
                <td><span class="pill" :class="item.status">{{ item.status }}</span><small v-if="item.error" class="error-text">{{ item.error }}</small></td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="empty-state compact">
          <strong>Aucun fichier correspondant</strong>
          <p>La source a été analysée sans produire de mouvement.</p>
        </div>
        <div v-if="report.scan_errors.length" class="error-box">
          <strong>Erreurs d’analyse</strong>
          <p v-for="error in report.scan_errors" :key="error">{{ error }}</p>
        </div>
      </article>
    </template>

    <div v-else class="empty-state panel roomy">
      <div class="empty-icon">P</div>
      <h2>Générez un plan avant toute action</h2>
      <p>Les collisions, règles gagnantes et destinations seront visibles ici.</p>
    </div>
  </section>
</template>
