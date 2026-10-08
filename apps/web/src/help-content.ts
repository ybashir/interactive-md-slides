export interface HelpBlock {
  type: 'paragraph' | 'list' | 'code' | 'note'
  text?: string
  items?: string[]
  code?: string
  language?: string
}

export interface HelpArticle {
  id: string
  title: string
  summary: string
  keywords: string[]
  blocks: HelpBlock[]
}

export interface InteractionOptionReference {
  name: string
  values: string
  defaultValue: string
  description: string
}

export interface InteractionReference {
  id: string
  name: string
  type: string
  syntax: string
  body: string
  options: InteractionOptionReference[]
}

export const commonInteractionOptions: InteractionOptionReference[] = [
  { name: 'type', values: 'One supported interaction type', defaultValue: 'Required', description: 'Selects the response contract. Use the exact type shown for each interaction below.' },
  { name: 'id', values: '2–64 letters, digits, _ or -; starts with a letter or digit', defaultValue: 'Required', description: 'Stable deck-wide interaction identity. Never reuse an ID for a different question.' },
  { name: 'question', values: 'Any short text', defaultValue: 'Slide heading', description: 'Overrides the audience prompt. It remains hidden on the projected slide unless show-title is true.' },
  { name: 'show-title', values: 'true | false', defaultValue: 'false', description: 'Shows the resolved interaction question as a projected interaction heading. Use it when the interaction itself should supply the visible prompt.' },
  { name: 'results', values: 'after-vote | always | live | manual | on-close | presenter | hidden', defaultValue: 'after-vote', description: 'Controls aggregate visibility. live is an alias of always; presenter and hidden never expose aggregates to the audience.' },
  { name: 'timer', values: 'Whole seconds from 5 to 7200', defaultValue: 'No timer', description: 'Adds presenter-controlled start, pause/resume, and reset timing.' },
  { name: 'display', values: 'card | slide', defaultValue: 'card', description: 'card uses the standard embedded treatment; slide lets the interaction become the primary slide content.' },
]

export const interactionReferences: InteractionReference[] = [
  {
    id: 'poll-options', name: 'Single- or multiple-choice poll', type: 'poll',
    syntax: ':::interact{type="poll" id="priority" multiple="true" max="2" results="after-vote"}',
    body: 'At least two options using - [stable-id] Visible label. There is no parser maximum, but keep projected choices concise.',
    options: [
      { name: 'multiple', values: 'true | false', defaultValue: 'false', description: 'Allows more than one selected option. Supplying max also enables multiple choice.' },
      { name: 'max', values: `Whole number from 1 to the interaction's option count`, defaultValue: 'All options', description: 'Maximum selections in a multiple-choice poll.' },
    ],
  },
  {
    id: 'quiz-options', name: 'Quiz', type: 'quiz',
    syntax: ':::interact{type="quiz" id="quick-check" correct="quality" results="manual" timer="30"}',
    body: 'At least two options using stable IDs. The correct value must match one of those IDs.',
    options: [
      { name: 'correct', values: 'One option ID', defaultValue: 'No scored answer', description: 'Marks the correct choice and enables correct/incorrect feedback.' },
    ],
  },
  {
    id: 'reaction-options', name: 'Slide-level emoji reaction', type: 'reaction',
    syntax: ':::interact{type="reaction" id="room-reaction" results="always"}',
    body: 'At least two options using - [stable-id] Emoji and label. Keep the set small enough to scan quickly.',
    options: [],
  },
  {
    id: 'word-cloud-options', name: 'Word cloud', type: 'word-cloud',
    syntax: ':::interact{type="word-cloud" id="check-in" entries="multiple" height="440" width="100%" spacing="0.22"}',
    body: 'No option list. Each entry may contain up to 80 characters. Multiple mode stores at most 50 unique, case-insensitive entries per participant.',
    options: [
      { name: 'entries', values: 'one | multiple', defaultValue: 'multiple', description: 'one replaces the participant’s prior entry; multiple permits several entries.' },
      { name: 'height', values: '2–4 digit pixel number', defaultValue: '320', description: 'Cloud canvas height. Invalid values fall back to 320px.' },
      { name: 'width', values: 'Percentage from 0% to 999%', defaultValue: '100%', description: 'Cloud canvas width. Use 100% for normal responsive slides.' },
      { name: 'spacing', values: 'Decimal; rendered range clamps to 0.04–0.5', defaultValue: '0.16', description: 'Padding between packed words. Increase it when words feel crowded.' },
    ],
  },
  {
    id: 'free-text-options', name: 'Free text', type: 'free-text',
    syntax: ':::interact{type="free-text" id="reflection" results="presenter"}',
    body: 'No option list. Each participant submits one response of 1–1,000 characters and may replace it.',
    options: [],
  },
  {
    id: 'rating-options', name: 'Rating scale', type: 'rating',
    syntax: ':::interact{type="rating" id="confidence" min="1" max="5"}',
    body: 'No option list. The audience selects one whole-number value from the inclusive range.',
    options: [
      { name: 'min', values: 'Whole number below max', defaultValue: '1', description: 'Lowest selectable rating.' },
      { name: 'max', values: 'Whole number above min; max - min ≤ 10', defaultValue: '5', description: 'Highest selectable rating.' },
    ],
  },
  {
    id: 'number-options', name: 'Numeric estimate', type: 'number',
    syntax: ':::interact{type="number" id="estimate" min="0" max="100" step="1" unit="days"}',
    body: 'No option list. The audience submits one finite number within the inclusive bounds.',
    options: [
      { name: 'min', values: 'Any finite number below max', defaultValue: '0', description: 'Minimum accepted value.' },
      { name: 'max', values: 'Any finite number above min', defaultValue: '100', description: 'Maximum accepted value.' },
      { name: 'step', values: 'Positive finite number', defaultValue: '1', description: 'Increment used by the audience input.' },
      { name: 'unit', values: 'Any short text', defaultValue: 'None', description: 'Displayed beside the numeric value, for example %, days, or people.' },
    ],
  },
  {
    id: 'allocation-options', name: 'Point allocation', type: 'allocation',
    syntax: ':::interact{type="allocation" id="investment" total="100" results="after-vote"}',
    body: 'At least two stable-ID options. Every allocation is a non-negative whole number and all values must sum to total.',
    options: [
      { name: 'total', values: 'Whole number from 1 to 1000', defaultValue: '100', description: 'Points each participant must distribute across all options.' },
    ],
  },
  {
    id: 'matrix-options', name: 'Two-axis matrix', type: 'matrix',
    syntax: ':::interact{type="matrix" id="priority" x-min="0" x-max="10" x-label="Effort" y-min="0" y-max="10" y-label="Impact" step="1"}',
    body: 'No option list. Each response is one finite X/Y point inside both inclusive ranges.',
    options: [
      { name: 'x-min', values: 'Any finite number below x-max', defaultValue: '0', description: 'Minimum horizontal-axis value.' },
      { name: 'x-max', values: 'Any finite number above x-min', defaultValue: '10', description: 'Maximum horizontal-axis value.' },
      { name: 'x-label', values: 'Any short text', defaultValue: 'X', description: 'Horizontal-axis label.' },
      { name: 'y-min', values: 'Any finite number below y-max', defaultValue: '0', description: 'Minimum vertical-axis value.' },
      { name: 'y-max', values: 'Any finite number above y-min', defaultValue: '10', description: 'Maximum vertical-axis value.' },
      { name: 'y-label', values: 'Any short text', defaultValue: 'Y', description: 'Vertical-axis label.' },
      { name: 'step', values: 'Positive finite number', defaultValue: '1', description: 'Increment used by both audience range inputs.' },
    ],
  },
  {
    id: 'image-choice-options', name: 'Image choice', type: 'image-choice',
    syntax: ':::interact{type="image-choice" id="direction" results="after-vote"}',
    body: 'At least two options using - [stable-id] Label {image="URL"}. Every image is required and must be HTTPS or an exact uploaded deck-asset URL.',
    options: [
      { name: 'image (per option)', values: 'https://… | /api/decks/…/assets/…/content', defaultValue: 'Required', description: 'Image displayed for that choice.' },
    ],
  },
  {
    id: 'ranking-options', name: 'Audience ranking', type: 'ranking',
    syntax: ':::interact{type="ranking" id="priority-order" results="after-vote"}',
    body: 'Between 2 and 20 stable-ID options. Every response must contain every option exactly once.',
    options: [],
  },
  {
    id: 'hotspot-options', name: 'Image hotspot', type: 'image-hotspot',
    syntax: ':::interact{type="image-hotspot" id="focus-map" image="https://example.com/map.png" alt="Office floor plan"}',
    body: 'No option list. The audience places one normalized point on the image.',
    options: [
      { name: 'image', values: 'https://… | /api/decks/…/assets/…/content', defaultValue: 'Required', description: 'Safe HTTPS image or exact uploaded deck-asset URL.' },
      { name: 'alt', values: 'Meaningful descriptive text', defaultValue: 'Required', description: 'Accessible description of the image and task context.' },
    ],
  },
  {
    id: 'survey-options', name: 'Multi-question survey', type: 'survey',
    syntax: ':::interact{type="survey" id="team-pulse" results="presenter"}',
    body: 'Contains 1–10 stable-ID questions. Each line ends with a rating, choice, or text configuration.',
    options: [
      { name: 'type (per question)', values: 'rating | choice | text', defaultValue: 'Required', description: 'Selects the answer shape for that survey question.' },
      { name: 'required (per question)', values: 'true | false', defaultValue: 'true', description: 'Whether the participant must answer the question.' },
      { name: 'min / max (rating)', values: 'Whole numbers; min < max and max - min ≤ 10', defaultValue: '1 / 5', description: 'Inclusive survey rating range.' },
      { name: 'options (choice)', values: '2–10 id:Label pairs separated by |', defaultValue: 'Required', description: 'Stable unique choices, for example quality:Quality|speed:Speed.' },
      { name: 'max (text)', values: 'Whole number from 1 to 1000', defaultValue: '500', description: 'Maximum text length.' },
    ],
  },
  {
    id: 'ranked-list-options', name: 'Live ranked list', type: 'ranked-list',
    syntax: ':::interact{type="ranked-list" id="ideas" display="slide" reveal="click" results="on-close"}',
    body: 'At least two stable-ID ideas. Participants may upvote or downvote revealed ideas; closing voting produces the final score order.',
    options: [
      { name: 'reveal', values: 'all | click', defaultValue: 'all', description: 'click exposes ideas to the audience only as the presenter reveals them.' },
      { name: 'display', values: 'card | slide', defaultValue: 'card', description: 'Use slide when the ranked list is the primary slide content.' },
    ],
  },
  {
    id: 'element-updown-options', name: 'Element up/down vote', type: 'updown',
    syntax: '- An idea {interact="updown" id="idea-vote"}',
    body: 'Attach to one Markdown list item. Each participant selects +1 or -1 and may change their vote.',
    options: [
      { name: 'interact', values: 'updown', defaultValue: 'Required', description: 'vote remains a legacy alias; use updown for new decks.' },
      { name: 'id', values: '2–64 letters, digits, _ or -', defaultValue: 'Required', description: 'Stable deck-wide identity.' },
    ],
  },
  {
    id: 'element-reaction-options', name: 'Element emoji reaction', type: 'reaction',
    syntax: '- A milestone {interact="reaction" id="milestone" options="like:👍|celebrate:🎉|question:🤔"}',
    body: 'Attach to one Markdown list item. The audience selects one reaction and may change it.',
    options: [
      { name: 'interact', values: 'reaction', defaultValue: 'Required', description: 'Selects element reaction behavior.' },
      { name: 'id', values: '2–64 letters, digits, _ or -', defaultValue: 'Required', description: 'Stable deck-wide identity.' },
      { name: 'options', values: '2–8 id:Emoji pairs separated by |', defaultValue: 'Required', description: 'Stable reaction IDs and their visible emoji labels.' },
    ],
  },
  {
    id: 'element-rating-options', name: 'Element rating', type: 'rating',
    syntax: '- Confidence in the plan {interact="rating" id="plan-confidence" min="1" max="5"}',
    body: 'Attach to one Markdown list item. The audience selects one whole-number value.',
    options: [
      { name: 'interact', values: 'rating', defaultValue: 'Required', description: 'Selects element rating behavior.' },
      { name: 'id', values: '2–64 letters, digits, _ or -', defaultValue: 'Required', description: 'Stable deck-wide identity.' },
      { name: 'min', values: 'Whole number below max', defaultValue: '1', description: 'Lowest selectable value.' },
      { name: 'max', values: 'Whole number above min; max - min ≤ 10', defaultValue: '5', description: 'Highest selectable value.' },
    ],
  },
]

export const helpArticles: HelpArticle[] = [
  {
    id: 'getting-started',
    title: 'Getting started',
    summary: 'Create a deck, understand the editor, and reach your first presentation.',
    keywords: ['new deck', 'preview', 'save', 'present', 'workflow'],
    blocks: [
      { type: 'paragraph', text: 'Create a deck from the dashboard, import an existing Slidev Markdown file, or open the Amazing Sea Creatures sample to explore a complete interactive presentation. Interdeck saves edits automatically and keeps the preview synchronized with the latest valid source.' },
      { type: 'list', items: [
        'Write or ask the Agent to change the current slide.',
        'Use + Add for interactions, charts, assets, and the audience QR code.',
        'Wait for “Draft saved” and confirm the live preview.',
        'Select Present, review the preflight, and start the presentation.',
        'Share only the stage window. Keep the presenter console private for controls and moderation.',
      ] },
      { type: 'note', text: 'Audience input is accepted only while a live or test presentation is running. Previewing a deck does not open interactions.' },
    ],
  },
  {
    id: 'editor',
    title: 'Editor modes and source structure',
    summary: 'Choose the smallest editing scope and understand headmatter, frontmatter, and stable IDs.',
    keywords: ['current slide', 'all slides', 'style.css', 'headmatter', 'frontmatter', 'marker', 'id'],
    blocks: [
      { type: 'list', items: [
        'Current slide is the default and shows only the slide in the preview. Slide-specific frontmatter is editable on slides after the first.',
        'All slides shows the complete slides.md file. Use it for deck headmatter, moving slides, or changes across several slides.',
        'Deck styles shows style.css and applies CSS across the entire deck.',
        'Theme choices live in the same Edit menu and do not change the selected editing scope.',
      ] },
      { type: 'paragraph', text: 'The first YAML block is deck headmatter. It configures the whole deck and is edited under All slides. Later YAML blocks belong to individual slides and can set layout, class, background, transition, and other Slidev properties.' },
      { type: 'code', language: 'md', code: `---
theme: seriph
title: "Quarterly review: Q2"
transition: slide-left
---

<!-- interdeck-slide: welcome -->
# Quarterly review

---
layout: center
class: text-center
---

<!-- interdeck-slide: closing -->
# Thank you` },
      { type: 'note', text: 'Keep one blank line around slide separators. Interdeck protects stable slide markers in Current slide mode because interactions and live state use those IDs.' },
    ],
  },
  {
    id: 'slidev',
    title: 'Slidev essentials',
    summary: 'Use Markdown, layouts, slots, speaker notes, and click-based reveals.',
    keywords: ['slidev', 'markdown', 'layout', 'two columns', 'v-click', 'v-clicks', 'speaker notes', 'animation'],
    blocks: [
      { type: 'paragraph', text: 'Ordinary Markdown creates slide content. A line containing --- separates slides. Interdeck supports the common Slidev layouts, slots, transitions, safe HTML, fenced code, and click reveals in the universal player.' },
      { type: 'code', language: 'md', code: `---
layout: two-cols-header
---

# Two perspectives

::left::

## Today
<v-clicks>

- Reliable delivery
- Growing capability

</v-clicks>

::right::

## Next
<v-clicks>

- Smaller changes
- Faster feedback

</v-clicks>` },
      { type: 'list', items: [
        'Use <v-click> for one reveal and <v-clicks> for a sequence of direct children.',
        'Avoid nesting v-clicks when a single ordered reveal sequence is sufficient.',
        'Put speaker notes in ordinary HTML comments at the end of a slide.',
        'Use explicit CSS rather than depending on arbitrary UnoCSS utilities in the universal player.',
      ] },
    ],
  },
  {
    id: 'styling',
    title: 'Styling slides',
    summary: 'Style one slide with scoped CSS or establish a deck-wide visual system.',
    keywords: ['css', 'style', 'scoped', 'font', 'size', 'center', 'image', 'theme'],
    blocks: [
      { type: 'paragraph', text: 'Use a scoped style block when a rule belongs to one slide. Use Deck styles when typography, spacing, colors, or component treatment should apply everywhere.' },
      { type: 'code', language: 'md', code: `# A focused statement

<div class="key-message">Small changes compound.</div>

<style scoped>
.key-message {
  margin-top: 2rem;
  font-size: 2.2rem;
  font-weight: 700;
  text-align: center;
}
</style>` },
      { type: 'code', language: 'css', code: `.slidev-layout h1 {
  letter-spacing: -0.04em;
}

.slidev-layout > ul {
  font-size: 1.2rem;
  line-height: 1.55;
}` },
      { type: 'note', text: 'If a style appears not to apply, check whether the selector matches the rendered Slidev structure and whether scoped CSS was placed on the intended slide.' },
    ],
  },
  {
    id: 'agent',
    title: 'Using the slide Agent',
    summary: 'Ask for focused changes, review the proposal, and keep requests fast and predictable.',
    keywords: ['agent', 'gemini', 'assistant', 'prompt', 'proposal', 'apply', 'discard', 'context'],
    blocks: [
      { type: 'paragraph', text: 'The Agent can create, rewrite, organize, style, and repair slides. It knows the supported Slidev and Interdeck syntax, the current preview slide, the deck asset catalog, and recent conversation.' },
      { type: 'list', items: [
        'Use Current slide for quick, focused requests such as “make this comparison reveal one card at a time.”',
        'Use All slides for deck-wide work, moving slides, changing headmatter, or creating a complete narrative.',
        'Review every proposed change before selecting Apply to deck. Discard leaves the source untouched.',
        'Refer to uploaded assets by their exact filename. The Agent will use the private URL from the deck catalog.',
        'The Agent edits slides only; it cannot start presentations, reset results, moderate questions, or change another deck.',
      ] },
      { type: 'note', text: 'A precise request containing the intended outcome and slide context is faster than asking for a broad redesign. Shift+Enter adds a line; Enter submits.' },
    ],
  },
  {
    id: 'assets',
    title: 'Assets and images',
    summary: 'Upload private images, SVGs, and fonts and insert their generated URLs safely.',
    keywords: ['asset', 'image', 'svg', 'png', 'jpeg', 'webp', 'font', 'upload', 'url'],
    blocks: [
      { type: 'paragraph', text: 'Open + Add → Asset to upload or insert a deck asset. Interdeck stores assets privately and generates the correct deck-scoped content URL. Do not invent /assets/filename paths.' },
      { type: 'code', language: 'html', code: `<img
  src="/api/decks/DECK_ID/assets/ASSET_ID/content"
  alt="Architecture lifecycle"
  style="display:block; width:500px; margin:1.5rem auto 0;"
>` },
      { type: 'list', items: [
        'Always provide useful alt text for meaningful images.',
        'Use display:block and auto side margins to center an image reliably.',
        'Use object-fit: contain when an image must fit a fixed region without cropping.',
        'Select the asset in Interdeck and let the editor insert its exact private URL.',
      ] },
    ],
  },
  {
    id: 'interactions',
    title: 'Interaction fundamentals',
    summary: 'Add slide-level interactions with stable IDs and control when results appear.',
    keywords: ['interaction', 'poll', 'results', 'stable id', 'open', 'close', 'audience'],
    blocks: [
      { type: 'paragraph', text: 'Interactions become available when their slide is active during a presentation. The slide heading is the audience prompt by default. Interaction titles stay hidden on the projected slide unless show-title="true" is set, so existing decks never gain a duplicate heading.' },
      { type: 'code', language: 'md', code: `# What should we prioritize?

:::interact{type="poll" id="roadmap-priority" results="after-vote"}
- [quality] Quality
- [speed] Delivery speed
- [learning] Learning
:::` },
      { type: 'list', items: [
        'Every interaction and option needs a stable, unique kebab-case ID.',
        'Use results="after-vote" for ordinary feedback, manual for presenter reveal, on-close for ranked results, and presenter for sensitive text.',
        'Starting another presentation resumes the current result epoch. Reset interaction data starts a new empty epoch.',
        'Do not reuse an interaction ID for a materially different question after responses exist.',
      ] },
    ],
  },
  {
    id: 'interaction-catalog',
    title: 'Interaction catalog',
    summary: 'Choose the right input type for the question you want the room to answer.',
    keywords: ['quiz', 'word cloud', 'rating', 'number', 'allocation', 'matrix', 'ranking', 'survey', 'hotspot', 'free text'],
    blocks: [
      { type: 'list', items: [
        'Poll — one choice; add multiple="true" max="2" for multiple choice.',
        'Quiz — scored choice with correct="option-id" and an optional timer.',
        'Reaction — one emoji response from a configured set.',
        'Word cloud — short text with entries="one" or entries="multiple".',
        'Free text — longer answers, normally hidden until the presenter chooses to show them.',
        'Rating — a bounded numeric scale such as 1–5.',
        'Number — numeric estimates with min, max, step, and optional unit.',
        'Allocation — distribute a fixed total, usually 100 points, across options.',
        'Matrix — place an answer on labelled X and Y axes.',
        'Image choice — choose among image-backed options.',
        'Ranking — order every option by preference.',
        'Ranked list — reveal and vote on ideas, then reorder when voting closes.',
        'Image hotspot — select a position on an image.',
        'Survey — combine 1–10 rating, choice, and text questions in one interaction.',
      ] },
      { type: 'code', language: 'md', code: `# Where should we invest?

:::interact{type="allocation" id="investment" total="100" results="after-vote"}
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::

---

# How confident are you?

:::interact{type="rating" id="confidence" min="1" max="5"}
:::` },
    ],
  },
  {
    id: 'interaction-options',
    title: 'Complete interaction option reference',
    summary: 'Every supported attribute, allowed value, default, body rule, and validation limit.',
    keywords: ['attributes', 'options', 'values', 'defaults', 'limits', 'reference', ...interactionReferences.flatMap(item => [item.name, item.type, item.syntax])],
    blocks: [
      { type: 'paragraph', text: 'All slide-level interaction blocks support the common attributes first. Each interaction card then lists every additional type-specific attribute and its exact accepted range.' },
      { type: 'note', text: 'Attributes not listed here are not part of Interdeck’s supported authoring contract, even if an underlying renderer happens to ignore or preserve them.' },
    ],
  },
  {
    id: 'element-interactions',
    title: 'Element-level voting and reactions',
    summary: 'Let the room respond directly to individual bullets.',
    keywords: ['element', 'bullet', 'upvote', 'downvote', 'updown', 'reaction', 'rating'],
    blocks: [
      { type: 'code', language: 'md', code: `# Vote on the ideas

- Ship smaller changes {interact="updown" id="smaller-changes"}
- Improve developer experience {interact="updown" id="developer-experience"}
- Celebrate the milestone {interact="reaction" id="milestone" options="like:👍|celebrate:🎉|question:🤔"}` },
      { type: 'paragraph', text: 'Audience members see only bullets already revealed by the presenter. Reactions appear on the projected slide only after they receive a non-zero count. Upvotes and downvotes are displayed separately.' },
      { type: 'note', text: 'Use updown for new binary voting. vote remains only as a compatibility alias for existing decks.' },
    ],
  },
  {
    id: 'word-clouds',
    title: 'Word clouds',
    summary: 'Collect one or several words and tune the packed cloud for the available space.',
    keywords: ['word cloud', 'entries', 'multiple', 'spacing', 'height', 'colors', 'orientation'],
    blocks: [
      { type: 'code', language: 'md', code: `# How are you feeling?

:::interact{type="word-cloud" id="check-in" entries="multiple" height="440" spacing="0.22"}
:::` },
      { type: 'list', items: [
        'entries="one" allows one submitted word per participant.',
        'entries="multiple" allows up to 12 distinct submissions per participant; this is the default when omitted.',
        'Each word or short phrase is limited to 40 characters. The API rejects oversized, malformed, or control-character input before storage.',
        'height controls vertical room and width accepts a CSS width such as 100%.',
        'Increase spacing when dense words appear too close together.',
        'Word size is proportional to submission count; color and orientation are assigned consistently for each word.',
      ] },
    ],
  },
  {
    id: 'charts',
    title: 'Charts with Apache ECharts',
    summary: 'Use native declarative ECharts JSON for static or live-result charts.',
    keywords: ['chart', 'echarts', 'bar', 'line', 'pie', 'dataset', 'series', 'reveal', 'live results'],
    blocks: [
      { type: 'paragraph', text: 'Interdeck renders native ECharts option JSON as SVG. JSON must be declarative: no functions, event handlers, imports, comments, or trailing commas.' },
      { type: 'code', language: 'md', code: `:::echarts{height="340" reveal="series"}
{
  "color": ["#6755d9", "#20a474"],
  "tooltip": { "trigger": "axis" },
  "legend": { "bottom": 0 },
  "dataset": {
    "source": [
      ["Quarter", "Revenue", "Costs"],
      ["Q1", 12, 8],
      ["Q2", 18, 11],
      ["Q3", 27, 16]
    ]
  },
  "xAxis": { "type": "category" },
  "yAxis": { "type": "value" },
  "series": [{ "type": "line" }, { "type": "line" }]
}
:::` },
      { type: 'paragraph', text: 'reveal="series" shows the first series immediately and reveals each remaining series on a Slidev click. To visualize live results, add source="interaction-id" and omit static dataset.source.' },
    ],
  },
  {
    id: 'audience-entry',
    title: 'Audience QR and joined count',
    summary: 'Give the audience a durable room entry point and show live attendance.',
    keywords: ['qr', 'join', 'room code', 'audience count', 'people joined', 'rotate'],
    blocks: [
      { type: 'code', language: 'md', code: `# Join the conversation

::audience-qr{size="220"}

::audience-count{label="people joined"}` },
      { type: 'paragraph', text: 'The QR component resolves the current room code and includes the live joined count. The standalone audience count is useful when the QR and attendance should be positioned separately.' },
      { type: 'note', text: 'Rotate room invalidates the old audience URL and is unavailable during a live run. Existing QR components update automatically.' },
    ],
  },
  {
    id: 'presenting',
    title: 'Presenting and presenter controls',
    summary: 'Run preflight, share the clean stage, and keep operational controls private.',
    keywords: ['present', 'fullscreen', 'stage', 'console', 'preflight', 'close voting', 'rehearsal', 'test'],
    blocks: [
      { type: 'list', items: [
        'Present opens preflight without compiling the deck again when the current source is already verified.',
        'Start presentation opens a clean projection stage and a separate private presenter console.',
        'Share the stage window, not the console. Moderation, timers, interaction controls, and insights stay private.',
        'Use the console to close voting, reveal results, control timers, freeze input, and navigate slides.',
        'The Slidev control menu appears when the pointer moves into the lower-left activation area.',
        'Test presentation isolates rehearsal responses from live results.',
      ] },
      { type: 'note', text: 'Editing is locked during a live presentation. End the run before changing the deck or asking the Agent for edits.' },
    ],
  },
  {
    id: 'qa',
    title: 'Q&A and moderation',
    summary: 'Collect deck-wide or slide-specific questions and moderate them privately.',
    keywords: ['q&a', 'question', 'moderation', 'approve', 'reject', 'anonymous', 'ai grouped', 'verbatim', 'upvote'],
    blocks: [
      { type: 'code', language: 'md', code: `# Audience Q&A

::live-qa{show="open"}` },
      { type: 'paragraph', text: 'The projected Q&A component shows approved questions in one scrollable column. show="open" excludes answered questions; show="all" includes them.' },
      { type: 'list', items: [
        'Verbatim mode is the default and displays approved questions exactly as submitted.',
        'AI-grouped mode uses the latest presenter-requested brief to combine similar questions into representative questions.',
        'Named, novel, and highly upvoted questions receive higher deterministic priority; anonymous questions are still accepted.',
        'Questions may be deck-wide or attached to the slide active when they were submitted.',
        'During presentation, hover or keyboard-focus a projected question and choose ✓ Done to mark it answered directly on the slide; show="open" removes it immediately.',
        'Approval, rejection, pinning, reopening, archiving, and full question management remain available in the private presenter console.',
      ] },
    ],
  },
  {
    id: 'live-summary',
    title: 'Live interaction summary',
    summary: 'Create a scrollable report from the current result epoch without rewriting the deck.',
    keywords: ['live summary', 'report', 'epoch', 'results', 'q&a', 'scroll'],
    blocks: [
      { type: 'code', language: 'md', code: `# What the room told us

::live-summary` },
      { type: 'paragraph', text: 'The live summary combines every interaction with responses, approved Q&A, participant totals, leaders, averages, rankings, and top words. It reads the current result epoch at runtime and remains available after the presentation ends.' },
      { type: 'list', items: [
        'Ask the Agent to add the live summary slide after the presentation if you want to demonstrate it on demand.',
        'Use ::live-summary{show="all"} only when unanswered interactions should also appear.',
        'The report is presenter-scrollable when it exceeds one slide.',
        'Reset interaction data starts a new empty epoch; prior epochs remain available for audit and export.',
      ] },
    ],
  },
  {
    id: 'results',
    title: 'Results, reset, and export',
    summary: 'Download audience data, start a fresh epoch, or export the verified deck.',
    keywords: ['csv', 'reset', 'epoch', 'export', 'pdf', 'pptx', 'history'],
    blocks: [
      { type: 'list', items: [
        'Results CSV downloads current responses and Q&A for the active result epoch.',
        'Reset interaction data starts a fresh empty epoch. It does not erase the archived prior epoch.',
        'Export deck opens Slidev’s creator-only browser exporter for the current verified source.',
        'Presenting again without a reset resumes existing tallies.',
      ] },
      { type: 'note', text: 'Export is intentionally presenter-only. Audience members use the room URL and never need the deck URL.' },
    ],
  },
  {
    id: 'troubleshooting',
    title: 'Troubleshooting',
    summary: 'Recover from common source, preview, asset, and live-session problems.',
    keywords: ['error', 'compile', 'yaml', 'refresh', 'stuck', 'asset broken', 'restore', 'loading'],
    blocks: [
      { type: 'list', items: [
        'Invalid headmatter: quote titles or values containing colons, hashes, brackets, braces, or leading punctuation.',
        'Slides merge together: put --- on its own line with a blank line before and after it.',
        'Preview shows an older version: wait for Draft saved, then use Refresh preview. Restore last working deck only if the new source cannot be repaired.',
        'An image is broken: insert it from Assets and use the generated /api/decks/.../content URL.',
        'A style does not apply: confirm whether it belongs in the slide-local style block or Deck styles and inspect the selector scope.',
        'Audience cannot interact: confirm a presentation is running, the intended slide is active, input is not frozen, and voting has not been closed.',
        'Agent request is slow: switch to Current slide for focused work; use All slides only when the complete deck is required.',
        'Q&A remains pending: use the private moderation queue; provider failures leave questions available for manual review.',
      ] },
      { type: 'note', text: 'Browser-extension “port disconnected” messages are unrelated to Interdeck unless accompanied by an Interdeck API or preview error.' },
    ],
  },
]
