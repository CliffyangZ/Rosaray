import { defineConfig } from 'vitepress'

export default defineConfig({
  lang: 'zh-TW',
  title: 'Rosaray',
  description: '本機優先（local-first）口內影像研究工作站 — 文件',
  base: '/ToothAI_Project/',
  cleanUrls: true,
  head: [['link', { rel: 'icon', type: 'image/png', href: '/ToothAI_Project/icon.png' }]],

  themeConfig: {
    logo: '/icon.png',
    nav: [
      { text: '指南', link: '/guide/getting-started' },
      { text: '架構', link: '/architecture/overview' },
      { text: 'Data Layer 規格', link: '/features/data-layer/spec' },
    ],

    sidebar: {
      '/guide/': [
        {
          text: '指南',
          items: [
            { text: '簡介', link: '/guide/introduction' },
            { text: '快速開始', link: '/guide/getting-started' },
            { text: '功能', link: '/guide/features' },
            { text: '專案結構', link: '/guide/project-structure' },
            { text: '目前狀態', link: '/guide/current-status' },
          ],
        },
      ],
      '/architecture/': [
        {
          text: '架構總覽',
          items: [{ text: 'System Overview', link: '/architecture/overview' }],
        },
        {
          text: 'Core',
          items: [
            { text: 'Core', link: '/architecture/core' },
            { text: 'Event Bus', link: '/architecture/event-bus' },
          ],
        },
        {
          text: 'Algorithm Layer',
          items: [
            { text: 'Algorithm Layer', link: '/architecture/algorithm-layer' },
            { text: 'Algorithm Designer', link: '/architecture/algorithm-designer' },
            { text: 'Pipeline Definition', link: '/architecture/pipeline-definition' },
          ],
        },
        {
          text: 'Execution Layer',
          items: [
            { text: 'Execution Layer', link: '/architecture/execution-layer' },
            { text: 'Execution Engine', link: '/architecture/execution-engine' },
            { text: 'Runtime Adapter', link: '/architecture/runtime-adapter' },
          ],
        },
        {
          text: 'Algorithm Runtime',
          items: [
            { text: 'Algorithm Runtime', link: '/architecture/algorithm-runtime' },
            { text: 'CPU', link: '/architecture/cpu' },
            { text: 'GPU / CUDA', link: '/architecture/gpu-cuda' },
            { text: 'ONNX / Runtime', link: '/architecture/onnx-runtime' },
          ],
        },
        {
          text: 'Data Layer',
          items: [
            { text: 'Data Layer', link: '/architecture/data-layer' },
            { text: 'Data Engine', link: '/architecture/data-engine' },
            { text: 'Data Repository', link: '/architecture/data-repository' },
            { text: 'Memory Cache', link: '/architecture/memory-cache' },
            { text: 'Persistent Storage', link: '/architecture/persistent-storage' },
          ],
        },
        {
          text: 'Presentation Layer',
          items: [
            { text: 'Presentation Layer', link: '/architecture/presentation-layer' },
            { text: 'Presentation Engine', link: '/architecture/presentation-engine' },
            { text: 'Image Viewer', link: '/architecture/image-viewer' },
            { text: 'Overlay Renderer', link: '/architecture/overlay-renderer' },
            { text: 'Annotation Renderer', link: '/architecture/annotation-renderer' },
            { text: 'Measurement Renderer', link: '/architecture/measurement-renderer' },
            { text: '3D Renderer', link: '/architecture/3d-renderer' },
          ],
        },
        {
          text: '工作台',
          items: [{ text: 'Workstation GUI', link: '/architecture/workstation-gui' }],
        },
      ],
      '/features/': [
        {
          text: 'Data Layer 設計',
          items: [
            { text: '規格 (Spec)', link: '/features/data-layer/spec' },
            { text: '研究 (Research)', link: '/features/data-layer/research' },
            { text: '資料模型 (Data Model)', link: '/features/data-layer/data-model' },
            { text: '實作計畫 (Plan)', link: '/features/data-layer/plan' },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/CliffyangZ/ToothAI_Project' },
    ],

    search: {
      provider: 'local',
    },

    outline: { label: '本頁目錄', level: [2, 3] },
    docFooter: { prev: '上一頁', next: '下一頁' },
    sidebarMenuLabel: '目錄',
    returnToTopLabel: '回到頂端',
    darkModeSwitchLabel: '深色模式',
  },
})
