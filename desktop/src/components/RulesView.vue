<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { api, chooseDirectory, errorMessage } from "../api";
import type { Config, PlannedMove, Rule } from "../types";
import StringListInput from "./StringListInput.vue";

const props = defineProps<{ config: Config }>();
const emit = defineEmits<{ dirty: [] }>();

const selectedIndex = ref(0);
const selected = computed(() => props.config.rules[selectedIndex.value]);
const previousId = ref("");
const testName = ref("invoice-2026.pdf");
const testResult = ref<PlannedMove | null>(null);
const testError = ref("");
const testing = ref(false);

watch(
  () => props.config.rules.length,
  (length) => {
    if (selectedIndex.value >= length) selectedIndex.value = Math.max(0, length - 1);
  },
);

function uniqueId(prefix: string): string {
  const existing = new Set(props.config.rules.map((entry) => entry.id));
  let index = 1;
  let candidate = prefix;
  while (existing.has(candidate)) candidate = `${prefix}-${index++}`;
  return candidate;
}

function addRule() {
  const rule: Rule = {
    id: uniqueId("nouvelle-regle"),
    enabled: true,
    priority: 10,
    destination: "",
    match: {
      operator: "all",
      exact_filenames: ["exemple.txt"],
      filename_globs: [],
      filename_regexes: [],
      extensions: [],
      case_sensitive: false,
    },
  };
  props.config.rules.push(rule);
  selectedIndex.value = props.config.rules.length - 1;
  emit("dirty");
}

function removeRule() {
  if (!selected.value) return;
  if (!window.confirm(`Supprimer la règle « ${selected.value.id} » et ses associations ?`)) return;
  const id = selected.value.id;
  props.config.rules.splice(selectedIndex.value, 1);
  for (const directory of props.config.directories) {
    directory.rules = directory.rules.filter((ruleId) => ruleId !== id);
  }
  emit("dirty");
}

function rememberId() {
  previousId.value = selected.value?.id || "";
}

function replaceRuleReferences() {
  if (!selected.value || !previousId.value || previousId.value === selected.value.id) return;
  for (const directory of props.config.directories) {
    directory.rules = directory.rules.map((id) => (id === previousId.value ? selected.value.id : id));
  }
  previousId.value = selected.value.id;
}

async function testRuleSet() {
  testing.value = true;
  testError.value = "";
  testResult.value = null;
  try {
    testResult.value = await api.testFilename(props.config, testName.value);
  } catch (error) {
    testError.value = errorMessage(error);
  } finally {
    testing.value = false;
  }
}

async function pickDestination() {
  if (!selected.value) return;
  const path = await chooseDirectory();
  if (path) {
    selected.value.destination = path;
    emit("dirty");
  }
}
</script>

<template>
  <section class="view-stack">
    <header class="view-heading">
      <div>
        <p class="eyebrow">Moteur déterministe</p>
        <h1>Règles de tri</h1>
        <p class="subtitle">Priorité décroissante, puis ordre de déclaration. La première correspondance gagne.</p>
      </div>
      <button class="primary" @click="addRule">Ajouter une règle</button>
    </header>

    <div v-if="config.rules.length" class="editor-layout">
      <aside class="editor-list panel">
        <button
          v-for="(rule, index) in config.rules"
          :key="`${rule.id}-${index}`"
          class="editor-list-item"
          :class="{ selected: index === selectedIndex }"
          @click="selectedIndex = index"
        >
          <span class="priority-badge">{{ rule.priority }}</span>
          <span class="grow min-width-zero">
            <strong class="truncate">{{ rule.id || "Sans identifiant" }}</strong>
            <small class="truncate">{{ rule.destination || "Destination à définir" }}</small>
          </span>
          <span class="status-dot" :class="rule.enabled ? 'online' : 'muted'" />
        </button>
      </aside>

      <article v-if="selected" class="panel form-panel" @input="emit('dirty')" @change="emit('dirty')">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">Règle {{ selectedIndex + 1 }}</p>
            <h2>{{ selected.id || "Nouvelle règle" }}</h2>
          </div>
          <button class="danger text-button" @click="removeRule">Supprimer</button>
        </div>

        <div class="form-grid two-columns">
          <label>
            <span>Identifiant unique</span>
            <input v-model.trim="selected.id" autocomplete="off" @focus="rememberId" @change="replaceRuleReferences" />
          </label>
          <label>
            <span>Priorité</span>
            <input v-model.number="selected.priority" type="number" />
          </label>
          <label class="full-width">
            <span>Répertoire de destination absolu</span>
            <div class="path-picker">
              <input v-model.trim="selected.destination" placeholder="/home/alice/Documents" autocomplete="off" />
              <button class="secondary" type="button" @click="pickDestination">Parcourir</button>
            </div>
          </label>
          <label>
            <span>Combinaison des groupes</span>
            <select v-model="selected.match.operator">
              <option value="all">Tous les groupes (AND)</option>
              <option value="any">Au moins un groupe (OR)</option>
            </select>
          </label>
          <label class="switch-row">
            <span>
              <strong>Sensible à la casse</strong>
              <small>Appliqué à toutes les conditions</small>
            </span>
            <input v-model="selected.match.case_sensitive" type="checkbox" class="switch" />
          </label>
          <label class="switch-row full-width">
            <span>
              <strong>Règle active</strong>
              <small>Une règle désactivée n’est jamais compilée</small>
            </span>
            <input v-model="selected.enabled" type="checkbox" class="switch" />
          </label>
        </div>

        <div class="condition-grid">
          <label>
            <span>Noms exacts <small>un par ligne</small></span>
            <StringListInput v-model="selected.match.exact_filenames" placeholder="rapport-final.pdf" />
          </label>
          <label>
            <span>Globs <small>un par ligne</small></span>
            <StringListInput v-model="selected.match.filename_globs" placeholder="facture-*.pdf" />
          </label>
          <label>
            <span>Expressions régulières <small>une par ligne</small></span>
            <StringListInput v-model="selected.match.filename_regexes" placeholder="^invoice-\d+\.pdf$" />
          </label>
          <label>
            <span>Extensions <small>une par ligne, sans point</small></span>
            <StringListInput v-model="selected.match.extensions" placeholder="pdf&#10;docx" />
          </label>
        </div>

        <div class="rule-tester">
          <div>
            <strong>Tester l’ensemble des règles</strong>
            <p>Le moteur Rust compile la configuration courante et applique les vraies priorités.</p>
          </div>
          <div class="test-controls">
            <input v-model.trim="testName" placeholder="nom-du-fichier.pdf" @keyup.enter="testRuleSet" />
            <button class="secondary" :disabled="testing || !testName" @click="testRuleSet">
              {{ testing ? "Test…" : "Tester" }}
            </button>
          </div>
          <p v-if="testError" class="error-text">{{ testError }}</p>
          <p v-else-if="testResult" class="success-text">
            Correspond à <strong>{{ testResult.rule_id }}</strong> → {{ testResult.destination }}
          </p>
          <p v-else-if="!testing" class="muted-text">Aucune correspondance pour le dernier test.</p>
        </div>
      </article>
    </div>

    <div v-else class="empty-state panel roomy">
      <div class="empty-icon">R</div>
      <h2>Aucune règle</h2>
      <p>Créez une règle explicite ; le moteur refusera toute condition vide ou invalide.</p>
      <button class="primary" @click="addRule">Créer la première règle</button>
    </div>
  </section>
</template>
