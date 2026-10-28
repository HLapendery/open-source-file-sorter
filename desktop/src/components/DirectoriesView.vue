<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { chooseDirectory } from "../api";
import { stateLabel, t } from "../i18n";
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
    id: uniqueId(t("directories.newId")),
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
  if (!window.confirm(t("directories.confirmDelete", { id: selected.value.id }))) return;
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
        <p class="eyebrow">{{ t("directories.eyebrow") }}</p>
        <h1>{{ t("directories.title") }}</h1>
        <p class="subtitle">{{ t("directories.subtitle") }}</p>
      </div>
      <button class="primary" @click="addDirectory">{{ t("directories.add") }}</button>
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
            <strong class="truncate">{{ directory.id || t("directories.unnamed") }}</strong>
            <small class="truncate">{{ directory.path || t("directories.pathMissing") }}</small>
          </span>
          <span v-if="isActive(directory.id)" class="pulse" :title="t('directories.activeAutomation')" />
        </button>
      </aside>

      <article v-if="selected" class="panel form-panel" @input="emit('dirty')" @change="emit('dirty')">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">{{ t("directories.source", { count: selectedIndex + 1 }) }}</p>
            <h2>{{ selected.id || t("directories.new") }}</h2>
          </div>
          <button class="danger text-button" @click="removeDirectory">{{ t("common.delete") }}</button>
        </div>

        <div class="form-grid two-columns">
          <label>
            <span>{{ t("directories.uniqueId") }}</span>
            <input v-model.trim="selected.id" autocomplete="off" />
          </label>
          <label class="switch-row">
            <span>
              <strong>{{ t("directories.active") }}</strong>
              <small>{{ t("directories.activeHint") }}</small>
            </span>
            <input v-model="selected.enabled" type="checkbox" class="switch" />
          </label>
          <label class="full-width">
            <span>{{ t("directories.sourcePath") }}</span>
            <div class="path-picker">
              <input v-model.trim="selected.path" placeholder="/home/alice/Downloads" autocomplete="off" />
              <button class="secondary" type="button" @click="pickSource">{{ t("common.browse") }}</button>
            </div>
          </label>
          <label>
            <span>{{ t("directories.executionMode") }}</span>
            <select v-model="selected.mode">
              <option value="once">{{ t("directories.modeOnce") }}</option>
              <option value="watch">{{ t("directories.modeWatch") }}</option>
              <option value="scheduled">{{ t("directories.modeScheduled") }}</option>
            </select>
          </label>
          <label class="switch-row">
            <span>
              <strong>{{ t("directories.recursive") }}</strong>
              <small>{{ t("directories.recursiveHint") }}</small>
            </span>
            <input v-model="selected.recursive" type="checkbox" class="switch" />
          </label>

          <template v-if="selected.mode === 'scheduled'">
            <label>
              <span>{{ t("directories.cron") }}</span>
              <input v-model.trim="selected.schedule" placeholder="0 0 2 * * *" autocomplete="off" />
            </label>
            <label>
              <span>{{ t("directories.timezone") }}</span>
              <select v-model="selected.schedule_timezone">
                <option value="local">{{ t("directories.localTime") }}</option>
                <option value="utc">UTC</option>
              </select>
            </label>
          </template>
        </div>

        <div class="field-group">
          <div>
            <strong>{{ t("directories.associatedRules") }}</strong>
            <p>{{ t("directories.priorityHint") }}</p>
          </div>
          <div v-if="config.rules.length" class="check-grid">
            <label v-for="rule in config.rules" :key="rule.id" class="check-card">
              <input v-model="selected.rules" type="checkbox" :value="rule.id" />
              <span>
                <strong>{{ rule.id }}</strong>
                <small>{{ t("directories.priority", { count: rule.priority }) }}</small>
              </span>
            </label>
          </div>
          <p v-else class="inline-warning">{{ t("directories.ruleRequired") }}</p>
        </div>

        <div v-if="selected.mode !== 'once'" class="automation-bar">
          <div>
            <strong>{{ t("directories.automation", { state: statusFor(selected.id)?.state ? stateLabel(statusFor(selected.id)!.state) : t("directories.stopped") }) }}</strong>
            <p v-if="dirty">{{ t("directories.saveBeforeStart") }}</p>
            <p v-else-if="statusFor(selected.id)?.error" class="error-text">{{ statusFor(selected.id)?.error }}</p>
            <p v-else>{{ t("directories.engineSafety") }}</p>
          </div>
          <button
            v-if="!isActive(selected.id)"
            class="secondary"
            :disabled="dirty || !selected.enabled"
            @click="emit('start', selected.id)"
          >
            {{ t("common.start") }}
          </button>
          <button v-else class="secondary" :disabled="statusFor(selected.id)?.state === 'stopping'" @click="emit('stop', selected.id)">
            {{ t("common.stop") }}
          </button>
        </div>
      </article>
    </div>

    <div v-else class="empty-state panel roomy">
      <div class="empty-icon">D</div>
      <h2>{{ t("directories.noSources") }}</h2>
      <p>{{ t("directories.noSourcesHint") }}</p>
      <button class="primary" @click="addDirectory">{{ t("directories.add") }}</button>
    </div>
  </section>
</template>
