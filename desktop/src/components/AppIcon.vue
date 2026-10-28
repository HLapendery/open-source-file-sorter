<script setup lang="ts">
defineProps<{ name: "overview" | "directories" | "rules" | "preview" | "history" | "language" }>();
</script>

<template>
  <svg v-if="name === 'overview'" viewBox="0 0 24 24" aria-hidden="true">
    <rect x="3" y="3" width="7" height="7" rx="2" />
    <rect x="14" y="3" width="7" height="7" rx="2" />
    <rect x="3" y="14" width="7" height="7" rx="2" />
    <rect x="14" y="14" width="7" height="7" rx="2" />
  </svg>
  <svg v-else-if="name === 'directories'" viewBox="0 0 24 24" aria-hidden="true">
    <path d="M3.5 7.5h6l2-2h4.2c1 0 1.8.8 1.8 1.8v1.2" />
    <path d="M4.8 8.5h14.4c1.1 0 1.9 1 1.6 2l-2 7.3c-.2.7-.9 1.2-1.6 1.2H5.8c-.8 0-1.5-.6-1.6-1.4L3.1 10.4c-.1-1 .7-1.9 1.7-1.9Z" />
  </svg>
  <svg v-else-if="name === 'rules'" viewBox="0 0 24 24" aria-hidden="true">
    <path d="M4 6h10M18 6h2M4 12h3M11 12h9M4 18h8M16 18h4" />
    <circle cx="16" cy="6" r="2" />
    <circle cx="9" cy="12" r="2" />
    <circle cx="14" cy="18" r="2" />
  </svg>
  <svg v-else-if="name === 'preview'" viewBox="0 0 24 24" aria-hidden="true">
    <path d="M2.7 12s3.4-6 9.3-6 9.3 6 9.3 6-3.4 6-9.3 6-9.3-6-9.3-6Z" />
    <circle cx="12" cy="12" r="2.8" />
  </svg>
  <svg v-else-if="name === 'history'" viewBox="0 0 24 24" aria-hidden="true">
    <path d="M4.7 7.5A8 8 0 1 1 4 15M4 4v4h4" />
    <path d="M12 8v4l2.8 1.8" />
  </svg>
  <svg v-else viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="12" cy="12" r="9" />
    <path d="M3.5 12h17M12 3c2.2 2.5 3.3 5.5 3.3 9S14.2 18.5 12 21c-2.2-2.5-3.3-5.5-3.3-9S9.8 5.5 12 3Z" />
  </svg>
</template>
