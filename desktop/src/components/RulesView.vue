<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { api, chooseDirectory, errorMessage } from "../api";
import { t } from "../i18n";
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
    id: uniqueId(t("rules.newId")),
    enabled: true,
    priority: 10,
    destination: "",
    match: {
      operator: "all",
      exact_filenames: [t("rules.exampleName")],
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
  if (!window.confirm(t("rules.confirmDelete", { id: selected.value.id }))) return;
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
        <p class="eyebrow">{{ t("rules.eyebrow") }}</p>
        <h1>{{ t("rules.title") }}</h1>
        <p class="subtitle">{{ t("rules.subtitle") }}</p>
      </div>
      <button class="primary" @click="addRule">{{ t("rules.add") }}</button>
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
            <strong class="truncate">{{ rule.id || t("rules.unnamed") }}</strong>
            <small class="truncate">{{ rule.destination || t("rules.destinationMissing") }}</small>
          </span>
          <span class="status-dot" :class="rule.enabled ? 'online' : 'muted'" />
        </button>
      </aside>

      <article v-if="selected" class="panel form-panel" @input="emit('dirty')" @change="emit('dirty')">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">{{ t("rules.rule", { count: selectedIndex + 1 }) }}</p>
            <h2>{{ selected.id || t("rules.new") }}</h2>
          </div>
          <button class="danger text-button" @click="removeRule">{{ t("common.delete") }}</button>
        </div>

        <div class="form-grid two-columns">
          <label>
            <span>{{ t("rules.uniqueId") }}</span>
            <input v-model.trim="selected.id" autocomplete="off" @focus="rememberId" @change="replaceRuleReferences" />
          </label>
          <label>
            <span>{{ t("rules.priority") }}</span>
            <input v-model.number="selected.priority" type="number" />
          </label>
          <label class="full-width">
            <span>{{ t("rules.destination") }}</span>
            <div class="path-picker">
              <input v-model.trim="selected.destination" placeholder="/home/alice/Documents" autocomplete="off" />
              <button class="secondary" type="button" @click="pickDestination">{{ t("common.browse") }}</button>
            </div>
          </label>
          <label>
            <span>{{ t("rules.combination") }}</span>
            <select v-model="selected.match.operator">
              <option value="all">{{ t("rules.all") }}</option>
              <option value="any">{{ t("rules.any") }}</option>
            </select>
          </label>
          <label class="switch-row">
            <span>
              <strong>{{ t("rules.caseSensitive") }}</strong>
              <small>{{ t("rules.caseSensitiveHint") }}</small>
            </span>
            <input v-model="selected.match.case_sensitive" type="checkbox" class="switch" />
          </label>
          <label class="switch-row full-width">
            <span>
              <strong>{{ t("rules.enabled") }}</strong>
              <small>{{ t("rules.enabledHint") }}</small>
            </span>
            <input v-model="selected.enabled" type="checkbox" class="switch" />
          </label>
        </div>

        <div class="condition-grid">
          <label>
            <span>{{ t("rules.exactNames") }} <small>{{ t("rules.onePerLine") }}</small></span>
            <StringListInput v-model="selected.match.exact_filenames" :placeholder="t('rules.exactPlaceholder')" />
          </label>
          <label>
            <span>{{ t("rules.globs") }} <small>{{ t("rules.onePerLine") }}</small></span>
            <StringListInput v-model="selected.match.filename_globs" :placeholder="t('rules.globPlaceholder')" />
          </label>
          <label>
            <span>{{ t("rules.regexes") }} <small>{{ t("rules.onePerLine") }}</small></span>
            <StringListInput v-model="selected.match.filename_regexes" placeholder="^invoice-\d+\.pdf$" />
          </label>
          <label>
            <span>{{ t("rules.extensions") }} <small>{{ t("rules.withoutDot") }}</small></span>
            <StringListInput v-model="selected.match.extensions" placeholder="pdf&#10;docx" />
          </label>
        </div>

        <div class="rule-tester">
          <div>
            <strong>{{ t("rules.testTitle") }}</strong>
            <p>{{ t("rules.testHint") }}</p>
          </div>
          <div class="test-controls">
            <input v-model.trim="testName" :placeholder="t('rules.testPlaceholder')" @keyup.enter="testRuleSet" />
            <button class="secondary" :disabled="testing || !testName" @click="testRuleSet">
              {{ testing ? t("rules.testing") : t("rules.test") }}
            </button>
          </div>
          <p v-if="testError" class="error-text">{{ testError }}</p>
          <p v-else-if="testResult" class="success-text">
            {{ t("rules.match", { rule: testResult.rule_id, destination: testResult.destination }) }}
          </p>
          <p v-else-if="!testing" class="muted-text">{{ t("rules.noMatch") }}</p>
        </div>
      </article>
    </div>

    <div v-else class="empty-state panel roomy">
      <div class="empty-icon">R</div>
      <h2>{{ t("rules.noRules") }}</h2>
      <p>{{ t("rules.noRulesHint") }}</p>
      <button class="primary" @click="addRule">{{ t("rules.createFirst") }}</button>
    </div>
  </section>
</template>
