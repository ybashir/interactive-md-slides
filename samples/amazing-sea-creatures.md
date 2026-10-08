---
theme: default
title: Amazing Sea Creatures
info: |
  A guided tour of ocean life and Interdeck's live presentation capabilities.
transition: slide-left
drawings:
  persist: false
layout: cover
class: sea-cover
background: /samples/amazing-sea-creatures/ocean-hero.webp
---
<!-- interdeck-slide: welcome-to-the-blue -->

<div class="sea-kicker">AN INTERACTIVE OCEAN EXPEDITION</div>

# Amazing Sea Creatures

## Meet the minds, lights, and giants beneath the surface.

::audience-qr{size="150"}

---
layout: fact
class: ocean-fact
---
<!-- interdeck-slide: another-planet -->

<div class="sea-kicker">THE BLUE PLANET</div>

# More than 70%

## of Earth is covered by ocean.

<v-click>

Yet most of that vast living world remains unseen by human eyes.

</v-click>

---
layout: two-cols-header
class: creature-profile
---
<!-- interdeck-slide: octopus-superpowers -->

# The octopus is built differently

::left::

<v-clicks>

- **Three hearts** keep oxygen moving {interact="reaction" id="octopus-hearts" options="wow:🤯|love:💙|curious:🤔"}
- **Blue blood** works in cold, low-oxygen water {interact="reaction" id="octopus-blood" options="wow:🤯|love:💙|curious:🤔"}
- **Distributed intelligence** puts neurons throughout its arms {interact="reaction" id="octopus-mind" options="wow:🤯|love:💙|curious:🤔"}

</v-clicks>

::right::

<figure class="sea-photo-card">
  <img src="/samples/amazing-sea-creatures/octopus.webp" alt="A giant Pacific octopus resting among rocks in a kelp forest">
  <figcaption>Giant Pacific octopus</figcaption>
</figure>

---
layout: statement
class: sea-photo-statement
background: /samples/amazing-sea-creatures/jellyfish.webp
---
<!-- interdeck-slide: living-light -->

<div class="sea-kicker">LIFE WITHOUT SUNLIGHT</div>

# Some animals make their own light.

<v-click>

Bioluminescence can attract prey, confuse predators, and help creatures find one another in the dark.

</v-click>

---
layout: center
class: interaction-stage
---
<!-- interdeck-slide: three-hearts-quiz -->

# Quick dive check

:::interact{type="quiz" id="three-hearts-quiz" question="Which sea creature has three hearts?" show-title="true" correct="octopus" results="manual" timer="20" display="slide"}
- [blue-whale] Blue whale
- [octopus] Octopus
- [sea-turtle] Green sea turtle
- [manta-ray] Manta ray
:::

---
layout: center
class: interaction-stage
---
<!-- interdeck-slide: choose-a-superpower -->

# Choose your ocean superpowers

:::interact{type="poll" id="ocean-superpowers" question="Which two adaptations would you borrow?" show-title="true" multiple="true" max="2" results="after-vote" display="slide"}
- [bioluminescence] Glow on demand
- [camouflage] Change color and texture
- [regeneration] Regrow a lost limb
- [echolocation] Navigate by sound
:::

---
layout: two-cols-header
class: cloud-expedition
---
<!-- interdeck-slide: deep-ocean-word-cloud -->

# The deep ocean feels…

::left::

## Add one word from your phone

Quiet, alien, beautiful, intimidating—there is no wrong answer.

<div class="sea-tip">Try several words. Repeated ideas grow larger in real time.</div>

::right::

:::interact{type="word-cloud" id="deep-ocean-feeling" question="Describe the deep ocean in one word" show-title="true" entries="multiple" results="always" height="330" width="100%" spacing="0.2" display="slide"}
:::

---
layout: center
class: interaction-stage
---
<!-- interdeck-slide: conservation-budget -->

# You have 100 conservation points

:::interact{type="allocation" id="conservation-budget" question="How should we invest them?" show-title="true" total="100" results="after-vote" display="slide"}
- [marine-reserves] Expand marine reserves
- [reef-restoration] Restore coral reefs
- [plastic-cleanup] Stop plastic at the source
- [ocean-research] Fund deep-ocean research
:::

---
layout: center
class: interaction-stage hotspot-stage
---
<!-- interdeck-slide: ocean-hotspot -->

# Pick the next expedition

:::interact{type="image-hotspot" id="ocean-expedition-map" question="Where would you explore first?" show-title="true" image="/samples/amazing-sea-creatures/ocean-hero.webp" alt="A humpback whale, sea turtle, tropical fish, and coral reef in clear blue ocean water" results="always" display="slide"}
:::

---
layout: center
class: interaction-stage
---
<!-- interdeck-slide: adaptation-rank -->

:::interact{type="ranked-list" id="adaptation-rank" question="Rank nature's wildest ideas" show-title="true" display="slide" reveal="click" results="on-close"}
- [octopus-camouflage] An octopus becoming part of the landscape
- [whale-song] A whale song travelling across the ocean
- [jellyfish-light] A jellyfish producing living light
- [turtle-navigation] A turtle returning to the beach where it hatched
:::

---
layout: two-cols-header
class: chart-expedition
---
<!-- interdeck-slide: pressure-and-light -->

# Down is a different world

::left::

## Two forces reshape life with depth

<v-clicks>

- Sunlight fades rapidly
- Pressure keeps climbing
- Adaptation becomes survival

</v-clicks>

::right::

:::echarts{height="350" reveal="series"}
{
  "color": ["#38bdf8", "#f59e0b"],
  "tooltip": { "trigger": "axis" },
  "legend": { "bottom": 0 },
  "grid": { "left": 48, "right": 30, "top": 28, "bottom": 58, "containLabel": true },
  "dataset": {
    "source": [
      ["Depth", "Sunlight", "Pressure"],
      ["Surface", 100, 1],
      ["200 m", 8, 21],
      ["500 m", 1, 51],
      ["1,000 m", 0, 101]
    ]
  },
  "xAxis": { "type": "category", "axisTick": { "show": false } },
  "yAxis": { "type": "value", "axisLine": { "show": false }, "splitLine": { "lineStyle": { "color": "#dbe7ee" } } },
  "series": [
    { "type": "line", "smooth": true, "symbolSize": 8, "lineStyle": { "width": 4 } },
    { "type": "line", "smooth": true, "symbolSize": 8, "lineStyle": { "width": 4 } }
  ]
}
:::

---
layout: center
class: interaction-stage
---
<!-- interdeck-slide: ocean-pulse -->

# A 30-second ocean pulse

:::interact{type="survey" id="ocean-pulse" question="Tell us where this expedition leaves you" show-title="true" results="always" display="slide"}
- [wonder] How much wonder did you feel? {type="rating" min="1" max="5"}
- [next-stop] Where should we dive next? {type="choice" options="reef:Coral reef|deep:Deep ocean|polar:Polar seas|kelp:Kelp forest"}
- [one-discovery] What is one thing you want to learn more about? {type="text" max="180" required="false"}
:::

---
layout: center
class: qa-expedition
---
<!-- interdeck-slide: audience-questions -->

# What are you curious about?

::live-qa{show="open" max="8"}

---
layout: center
class: summary-expedition
---
<!-- interdeck-slide: expedition-summary -->

# What this room discovered

::live-summary{show="responded"}
