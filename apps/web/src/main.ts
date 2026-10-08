import { createApp } from 'vue'
import { createRouter, createWebHistory } from 'vue-router'

import App from './App.vue'
import PresentView from './views/PresentView.vue'
import './styles.css'

const ROUTE_RELOAD_KEY = 'interdeck:stale-route-reload'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', component: () => import('./views/DashboardView.vue') },
    { path: '/help', component: () => import('./views/HelpCenterView.vue') },
    { path: '/privacy', component: () => import('./views/PrivacyView.vue') },
    { path: '/login', component: () => import('./views/LoginView.vue') },
    { path: '/decks/:id/player', component: () => import('./views/DeckPlayerView.vue') },
    // Presentation is a critical transition from a potentially long-lived
    // editor tab. Keep it in the loaded application bundle so a deployment
    // cannot remove the lazy chunk between opening the editor and presenting.
    { path: '/decks/:id/present', component: PresentView },
    { path: '/decks/:id', component: () => import('./views/EditorView.vue') },
    { path: '/j/:joinCode', component: () => import('./views/AudienceView.vue') },
  ],
})

router.onError((error, to) => {
  const message = error instanceof Error ? error.message : String(error)
  if (!/dynamically imported module|importing a module script failed|unable to preload css|loading chunk .* failed/i.test(message)) return

  const destination = to.fullPath || window.location.pathname + window.location.search
  const previous = window.sessionStorage.getItem(ROUTE_RELOAD_KEY)
  const [previousDestination, previousTime] = previous?.split('|') || []
  if (previousDestination === destination && Date.now() - Number(previousTime || 0) < 30_000) return

  window.sessionStorage.setItem(ROUTE_RELOAD_KEY, `${destination}|${Date.now()}`)
  window.location.assign(destination)
})

router.afterEach((to) => {
  const previous = window.sessionStorage.getItem(ROUTE_RELOAD_KEY)
  if (previous?.startsWith(`${to.fullPath}|`)) window.sessionStorage.removeItem(ROUTE_RELOAD_KEY)
})

createApp(App).use(router).mount('#app')
