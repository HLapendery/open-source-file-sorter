<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { chooseDirectory } from "../api";
import type { AutomationStatus, Config, DirectoryConfig } from "../types";

const props = defineProps<{
  config: Config;
  automation: AutomationStatus[];
  dirty: boolean;
}>();
const emit = defineEmits<{
  dirty: [];
  start: [directoryId: string];
  stop: [directoryId: string];
}>();

const selectedIndex = ref(0);
const selected = computed(() => props.config.directories[selectedIndex.value]);

watch(
  () => props.config.directories.length,
  (length) => {
    if (selectedIndex.value >= length) selectedIndex.value = Math.max(0, length - 1);
  },
);

function uniqueId(prefix: string): string {
  const existing = new Set(props.config.directories.map((entry) => entry.id));
  let index = 1;
  let candidate = prefix;
  while (existing.has(candidate)) candidate = `${prefix}-${index++}`;
  return candidate;
}

function addDirectory() {
  const directory: DirectoryConfig = {
    id: uniqueId("nouveau-repertoire"),
    path: "",
    enabled: true,
    mode: "once",
    recursive: false,
    schedule: null,
    schedule_timezone: "local",
    rules: props.config.rules[0] ? [props.config.rules[0].id] : [],
  };
  props.config.directories.push(directory);
  selectedIndex.value = props.config.directories.length - 1;
  emit("dirty");
}

function removeDirectory() {
  if (!selected.value) return;
  if (!window.confirm(`Supprimer la configuration « ${selected.value.id} » ? Aucun fichier ne sera supprimé.`)) return;
  props.config.directories.splice(selectedIndex.value, 1);
  emit("dirty");
}

function statusFor(id: string): AutomationStatus | undefined {
  return props.automation.find((entry) => entry.directory_id === id);
}

function isActive(id: string): boolean {
  const state = statusFor(id)?.state;
  return state === "running" || state === "stopping";
}

async function pickSource() {
  if (!selected.value) return;
  const path = await chooseDirectory();
  if (path) {
    selected.value.path = path;
    emit("dirty");
  }
}
</script>

<template>
  <section class="view-stack">
    <header class="view-heading">
      <div>
        <p class="eyebrow">Configuration</p>
        <h1>Répertoires</h1>
        <p class="subtitle">Chaque source garde son mode, sa récursivité et son jeu de règles.</p>
      </div>
      <button class="primary" @click="addDirectory">Ajouter un répertoire</button>
    </header>

    <div v-if="config.directories.length" class="editor-layout">
      <aside class="editor-list panel">
        <button
          v-for="(directory, index) in config.directories"
          :key="`${directory.id}-${index}`"
          class="editor-list-item"
          :class="{ selected: index === selectedIndex }"
          @click="selectedIndex = index"
        >
          <span class="status-dot" :class="directory.enabled ? 'online' : 'muted'" />
          <span class="grow min-width-zero">
            <strong class="truncate">{{ directory.id || "Sans identifiant" }}</strong>
            <small class="truncate">{{ directory.path || "Chemin à définir" }}</small>
          </span>
          <span v-if="isActive(directory.id)" class="pulse" title="Automatisation active" />
        </button>
      </aside>

      <article v-if="selected" class="panel form-panel" @input="emit('dirty')" @change="emit('dirty')">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">Source {{ selectedIndex + 1 }}</p>
            <h2>{{ selected.id || "Nouveau répertoire" }}</h2>
          </div>
          <button class="danger text-button" @click="removeDirectory">Supprimer</button>
        </div>

        <div class="form-grid two-columns">
          <label>
            <span>Identifiant unique</span>
            <input v-model.trim="selected.id" autocomplete="off" />
          </label>
          <label class="switch-row">
            <span>
              <strong>Configuration active</strong>
              <small>Incluse dans les exécutions</small>
            </span>
            <input v-model="selected.enabled" type="checkbox" class="switch" />
          </label>
          <label class="full-width">
            <span>Chemin source absolu</span>
            <div class="path-picker">
              <input v-model.trim="selected.path" placeholder="/home/alice/Downloads" autocomplete="off" />
              <button class="secondary" type="button" @click="pickSource">Parcourir</button>
            </div>
          </label>
          <label>
            <span>Mode d’exécution</span>
            <select v-model="selected.mode">
              <option value="once">Ponctuel</option>
              <option value="watch">Surveillance</option>
              <option value="scheduled">Planifié</option>
            </select>
          </label>
          <label class="switch-row">
            <span>
              <strong>Parcours récursif</strong>
              <small>Inclure les sous-répertoires</small>
            </span>
            <input v-model="selected.recursive" type="checkbox" class="switch" />
          </label>

          <template v-if="selected.mode === 'scheduled'">
            <label>
              <span>Expression cron (6 champs)</span>
              <input v-model.trim="selected.schedule" placeholder="0 0 2 * * *" autocomplete="off" />
            </label>
            <label>
              <span>Fuseau horaire</span>
              <select v-model="selected.schedule_timezone">
                <option value="local">Heure locale</option>
                <option value="utc">UTC</option>
              </select>
            </label>
          </template>
        </div>

        <div class="field-group">
          <div>
            <strong>Règles associées</strong>
            <p>La priorité globale décide de la première règle gagnante.</p>
          </div>
          <div v-if="config.rules.length" class="check-grid">
            <label v-for="rule in config.rules" :key="rule.id" class="check-card">
              <input v-model="selected.rules" type="checkbox" :value="rule.id" />
              <span>
                <strong>{{ rule.id }}</strong>
                <small>Priorité {{ rule.priority }}</small>
              </span>
            </label>
          </div>
          <p v-else class="inline-warning">Créez au moins une règle avant d’enregistrer ce répertoire.</p>
        </div>

        <div v-if="selected.mode !== 'once'" class="automation-bar">
          <div>
            <strong>Automatisation {{ statusFor(selected.id)?.state || "arrêtée" }}</strong>
            <p v-if="dirty">Enregistrez la configuration avant de la démarrer.</p>
            <p v-else-if="statusFor(selected.id)?.error" class="error-text">{{ statusFor(selected.id)?.error }}</p>
            <p v-else>Le moteur Rust conserve le journal et les garanties de sécurité.</p>
          </div>
          <button
            v-if="!isActive(selected.id)"
            class="secondary"
            :disabled="dirty || !selected.enabled"
            @click="emit('start', selected.id)"
          >
            Démarrer
          </button>
          <button v-else class="secondary" :disabled="statusFor(selected.id)?.state === 'stopping'" @click="emit('stop', selected.id)">
            Arrêter
          </button>
        </div>
      </article>
    </div>

    <div v-else class="empty-state panel roomy">
      <div class="empty-icon">D</div>
      <h2>Aucune source configurée</h2>
      <p>Ajoutez le premier répertoire à analyser. Aucun fichier n’est touché lors de la configuration.</p>
      <button class="primary" @click="addDirectory">Ajouter un répertoire</button>
    </div>
  </section>
</template>
