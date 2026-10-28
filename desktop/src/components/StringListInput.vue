<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{
  modelValue: string[];
  placeholder?: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();

const text = computed({
  get: () => props.modelValue.join("\n"),
  set: (value: string) =>
    emit(
      "update:modelValue",
      value
        .split(/\r?\n/)
        .map((entry) => entry.trim())
        .filter(Boolean),
    ),
});
</script>

<template>
  <textarea v-model="text" rows="3" :placeholder="placeholder" />
</template>
