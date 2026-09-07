import { layoutTabs } from "./api";
import { compareVersions, tagOf } from "./registry";
import "./styles.css";
import {
  TABBAR_H,
  badgeStatus as domainBadgeStatus,
  buildLayoutInput,
  desiredVersionOf as domainDesiredVersionOf,
  envById as domainEnvById,
  isUpdateAvailable as domainIsUpdateAvailable,
  showTabEnv,
  type EnvRow,
  type ViewMode,
} from "./domain/environments";
import { defaultGateway } from "./infra/envGateway";
import { EnvironmentService } from "./services/environmentService";
import { BrowserSettingsStorage } from "./services/settingsStore";
import type { RegistryData } from "./types";

// ---------------------------------------------------------------------------
// Stato applicativo (la forma EnvRow/ViewMode vive in domain/environments:
// main.ts e solo composition root + renderer DOM, non piu contenitore di logica).
// ---------------------------------------------------------------------------

const envService = new EnvironmentService(defaultGateway, new BrowserSettingsStorage());

let envs: EnvRow[] = [];
let registry: RegistryData | null = null;
let registryError: string | null = null;
let selectedId: string | null = null;
let scanning = false;
let statusMessage = "";
let layoutTimer: number | undefined;

let viewMode: ViewMode = { view: "settings" };

// ---------------------------------------------------------------------------
// Persistenza impostazioni: delegata a EnvironmentService/SettingsStore (DIP).
// ---------------------------------------------------------------------------

function saveSettings(): void {
  envService.persist(envs);
}

// ---------------------------------------------------------------------------
// Rilevamento ambienti
// ---------------------------------------------------------------------------

// Rilevamento ambienti: orchestrazione delegata a EnvironmentService (OCP/DIP).
// main.ts applica solo il risultato allo stato UI (selectedId/statusMessage).

async function scanEnvironments(showSpinner = true): Promise<void> {
  if (scanning) return;
  scanning = true;
  if (showSpinner) statusMessage = "Rilevamento ambienti in corso…";
  renderAll();
  try {
    const result = await envService.scanEnvironments(envs);
    envs = result.envs;
    if (result.status) statusMessage = result.status;
    if (!selectedId || !envs.some((e) => e.id === selectedId)) {
      selectedId = envs[0]?.id ?? null;
    }
    if (showSpinner) statusMessage = "";
  } finally {
    scanning = false;
    renderAll();
  }
}

async function refreshRunningStates(): Promise<void> {
  const changed = await envService.refreshRunningStates(envs);
  updateRunningBadges();
  if (changed) {
    // se l'ambiente della tab attiva non è più in esecuzione, torna alle impostazioni
    if (viewMode.view === "env" && !domainEnvById(envs, viewMode.envId)?.running) {
      viewMode = { view: "settings" };
      renderChrome();
    }
    void applyLayout();
  }
}

async function loadRegistry(): Promise<void> {
  const outcome = await envService.loadRegistry();
  registry = outcome.registry;
  registryError = outcome.error;
  renderDetail();
}

// ---------------------------------------------------------------------------
// Helper target & versioni
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Helper target & versioni: thin wrapper sul dominio (la regola vive in
// domain/environments; qui si lega solo il registry corrente della UI).
// ---------------------------------------------------------------------------

function desiredVersionOf(e: EnvRow): string | null {
  return domainDesiredVersionOf(e, registry);
}

function isUpdateAvailable(e: EnvRow): boolean {
  return domainIsUpdateAvailable(e, registry);
}

// ---------------------------------------------------------------------------
// Vista "browser": tab di DSH incorporate sotto la barra
// ---------------------------------------------------------------------------

function envById(id: string): EnvRow | undefined {
  return domainEnvById(envs, id);
}

/** Sincronizza le webview figlie (GUI DSH) con la tab attiva e le dimensioni. */
async function applyLayout(): Promise<void> {
  const activeEnv =
    viewMode.view === "env" && envById(viewMode.envId)?.running ? viewMode.envId : null;
  // Logica URL autenticato + finestra di grazia in domain/environments.
  const input = buildLayoutInput(
    envs,
    activeEnv,
    { tabbar: TABBAR_H, width: window.innerWidth, height: window.innerHeight },
    Date.now(),
  );
  try {
    await layoutTabs(input);
  } catch (err) {
    console.warn("layout_tabs:", err);
  }
}

/** Accorpa chiamate ravvicinate (es. resize). */
function scheduleLayout(): void {
  window.clearTimeout(layoutTimer);
  layoutTimer = window.setTimeout(() => void applyLayout(), 120);
}

/** Attiva la tab di un ambiente (mostra la GUI di DSH se in esecuzione). */
function showEnvTab(envId: string): void {
  const e = envById(envId);
  if (!e) return;
  selectedId = envId;
  if (e.running) {
    viewMode = { view: "env", envId };
  } else {
    // non in esecuzione: resta sulle impostazioni ma mostra il dettaglio
    viewMode = { view: "settings" };
  }
  renderChrome();
  renderEnvList();
  renderDetail();
  void applyLayout();
}

/** Attiva la schermata impostazioni (nasconde le webview di DSH). */
function showSettingsTab(): void {
  viewMode = { view: "settings" };
  renderChrome();
  void applyLayout();
}

// ---------------------------------------------------------------------------
// Azioni
// ---------------------------------------------------------------------------

// Azioni: main.ts gestisce solo busy/status/render; l'orchestrazione I/O
// (porte libere, start/stop/update, re-probe) vive in EnvironmentService.

async function actionStart(e: EnvRow): Promise<void> {
  e.busy = true;
  statusMessage = `Avvio di dsh su ${e.name} (porta ${e.settings.port})…`;
  renderAll();
  try {
    const outcome = await envService.startEnvironment(e);
    if (outcome.ok) {
      e.settings.port = outcome.port;
      saveSettings();
      e.running = outcome.reached;
      e.authUrl = outcome.authUrl;
      e.authSentAt = null;
    } else {
      e.authUrl = null;
      e.authSentAt = null;
    }
    statusMessage = outcome.message;
  } catch (err) {
    statusMessage = `Avvio fallito: ${String(err)}`;
  } finally {
    e.busy = false;
    renderAll();
    // GUI al centro dell'attenzione: apri subito la tab dell'ambiente
    if (e.running) {
      showEnvTab(e.id);
    } else {
      void applyLayout();
    }
  }
}

async function actionStop(e: EnvRow): Promise<void> {
  e.busy = true;
  statusMessage = `Arresto di dsh su ${e.name}…`;
  renderAll();
  try {
    const outcome = await envService.stopEnvironment(e);
    e.running = false;
    e.authUrl = null;
    e.authSentAt = null;
    statusMessage = outcome.message;
  } catch (err) {
    statusMessage = `Arresto fallito: ${String(err)}`;
  } finally {
    e.busy = false;
    if (viewMode.view === "env" && viewMode.envId === e.id) {
      viewMode = { view: "settings" };
    }
    renderAll();
    void applyLayout();
  }
}

async function actionUpdate(e: EnvRow): Promise<void> {
  const target = desiredVersionOf(e);
  if (!target) {
    statusMessage = "Nessuna versione selezionabile (registry non raggiungibile?).";
    renderAll();
    return;
  }
  e.busy = true;
  const verb = e.probe?.installed ? "Aggiornamento" : "Installazione";
  statusMessage = `${verb} di dsh su ${e.name} alla versione ${target}… (può richiedere alcuni minuti)`;
  renderAll();
  try {
    const outcome = await envService.updateEnvironment(e, registry);
    if (outcome.ok) {
      statusMessage = outcome.message;
      e.note = outcome.note;
      if (outcome.probe) e.probe = outcome.probe;
    } else {
      statusMessage = outcome.error;
    }
  } catch (err) {
    statusMessage = `${verb} fallito: ${String(err)}`;
  } finally {
    e.busy = false;
    renderAll();
  }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

const appEl = document.getElementById("app")!;
let headerBox: HTMLDivElement;

function renderAll(): void {
  renderChrome();
  renderEnvList();
  renderDetail();
}

/** Barra superiore "tipo browser": brand + tab ambienti + tab Impostazioni. */
function renderChrome(): void {
  const bar = document.createElement("div");
  bar.className = "tabbar";

  const brand = document.createElement("div");
  brand.className = "brand";
  brand.textContent = "DSH Manager";
  bar.appendChild(brand);

  const tabs = document.createElement("div");
  tabs.className = "tablist";

  for (const e of envs.filter(showTabEnv)) {
    const t = document.createElement("button");
    const active = viewMode.view === "env" && viewMode.envId === e.id;
    t.className = "tab" + (active ? " active" : "");
    t.title = `${e.name} — ${e.running ? `GUI su :${e.settings.port}` : "non in esecuzione"}`;
    const ic = document.createElement("span");
    ic.className = "tab-icon";
    ic.textContent = e.kind === "windows" ? "🪟" : "🐧";
    const nm = document.createElement("span");
    nm.className = "tab-name";
    nm.textContent = e.name;
    const dot = document.createElement("span");
    dot.className = "tab-dot " + (e.running ? "on" : "off");
    dot.dataset.tabFor = e.id;
    t.appendChild(ic);
    t.appendChild(nm);
    t.appendChild(dot);
    t.addEventListener("click", () => showEnvTab(e.id));
    tabs.appendChild(t);
  }

  const settingsTab = document.createElement("button");
  settingsTab.className = "tab" + (viewMode.view === "settings" ? " active" : "");
  settingsTab.title = "Impostazioni e gestione ambienti";
  const icS = document.createElement("span");
  icS.className = "tab-icon";
  icS.textContent = "⚙";
  const nmS = document.createElement("span");
  nmS.className = "tab-name";
  nmS.textContent = "Impostazioni";
  settingsTab.appendChild(icS);
  settingsTab.appendChild(nmS);
  settingsTab.addEventListener("click", () => showSettingsTab());
  tabs.appendChild(settingsTab);

  bar.appendChild(tabs);

  const right = document.createElement("div");
  right.className = "tabbar-right";
  const refresh = document.createElement("button");
  refresh.className = "icon-btn";
  refresh.title = "Rileva di nuovo gli ambienti";
  refresh.textContent = "⟳";
  refresh.disabled = scanning;
  refresh.addEventListener("click", () => void scanEnvironments(true));
  right.appendChild(refresh);
  bar.appendChild(right);

  headerBox.replaceChildren(bar);
}

// badgeStatus: wrapper UI sul dominio (lega il registry corrente). La regola
// vive in domain/environments cosi e testabile senza DOM. Il badge del
// dettaglio mostra anche "In esecuzione" ma senza il punto del badge lista
// (stesso testo storico del dettaglio: "Sì — porta N" e gestito a parte).
function badgeStatus(e: EnvRow): { text: string; cls: string } {
  return domainBadgeStatus(e, registry);
}

function renderEnvList(): void {
  const list = document.createElement("div");
  list.className = "env-list";
  for (const e of envs) {
    const card = document.createElement("div");
    card.className = "env-card" + (e.id === selectedId ? " selected" : "");
    card.dataset.envId = e.id;

    const icon = document.createElement("div");
    icon.className = "env-icon";
    icon.textContent = e.kind === "windows" ? "🪟" : "🐧";

    const main = document.createElement("div");
    main.className = "env-main";
    const nm = document.createElement("div");
    nm.className = "env-name";
    nm.textContent = e.name;
    const badge = badgeStatus(e);
    const st = document.createElement("div");
    st.className = `env-status ${badge.cls}`;
    st.dataset.statusFor = e.id;
    st.textContent = badge.text;
    main.appendChild(nm);
    main.appendChild(st);

    card.appendChild(icon);
    card.appendChild(main);
    card.addEventListener("click", () => {
      selectedId = e.id;
      renderEnvList();
      renderDetail();
    });
    list.appendChild(card);
  }
  const listBox = document.getElementById("envListBox");
  if (listBox) listBox.replaceChildren(list);
}

function updateRunningBadges(): void {
  for (const e of envs) {
    const badge = badgeStatus(e);
    const el = document.querySelector<HTMLElement>(`.env-status[data-status-for="${CSS.escape(e.id)}"]`);
    if (el) {
      const cls = `env-status ${badge.cls}`;
      // Scrivi nel DOM solo se cambia davvero: niente refresh visivi a ogni ciclo
      if (el.className !== cls) el.className = cls;
      if (el.textContent !== badge.text) el.textContent = badge.text;
    }
    const dot = document.querySelector<HTMLElement>(`.tab-dot[data-tab-for="${CSS.escape(e.id)}"]`);
    if (dot) {
      const want = `tab-dot ${e.running ? "on" : "off"}`;
      if (dot.className !== want) dot.className = want;
    }
    if (e.id === selectedId) {
      const runningEl = document.getElementById("detailRunning");
      if (runningEl) {
        const t = e.running ? `Sì — porta ${e.settings.port}` : "No";
        if (runningEl.textContent !== t) runningEl.textContent = t;
      }
    }
  }
}

function fmtRow(label: string, value: string | null | undefined, mono = false): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "info-row";
  const l = document.createElement("span");
  l.className = "info-label";
  l.textContent = label;
  const v = document.createElement("span");
  v.className = "info-value" + (mono ? " mono" : "");
  v.textContent = value && value.length > 0 ? value : "—";
  row.appendChild(l);
  row.appendChild(v);
  return row;
}

function renderDetail(): void {
  const box = document.getElementById("detailBox");
  if (!box) return;
  const e = envs.find((x) => x.id === selectedId);
  if (!e) {
    box.replaceChildren(emptyState());
    return;
  }

  const wrap = document.createElement("div");
  wrap.className = "detail";

  // Intestazione
  const head = document.createElement("div");
  head.className = "detail-head";
  const h = document.createElement("h2");
  h.textContent = e.name;
  const st = document.createElement("span");
  const badge = badgeStatus(e);
  st.className = `pill ${badge.cls}`;
  st.textContent = badge.text;
  st.id = "detailRunning";
  head.appendChild(h);
  head.appendChild(st);
  wrap.appendChild(head);

  // Rilevamento
  const sec1 = document.createElement("section");
  sec1.className = "section";
  const stitle = document.createElement("h3");
  stitle.textContent = "Rilevamento dsh";
  sec1.appendChild(stitle);
  const p = e.probe;
  if (p?.error && !p.installed) {
    const er = document.createElement("div");
    er.className = "warn-box";
    er.textContent = `Errore rilevamento: ${p.error}`;
    sec1.appendChild(er);
  }
  sec1.appendChild(fmtRow("Installato", p?.installed ? "Sì" : "No"));
  sec1.appendChild(fmtRow("Versione installata", p?.installed ? p?.version : null, true));
  sec1.appendChild(fmtRow("Eseguibile", p?.executable, true));
  sec1.appendChild(fmtRow("DSH_HOME", p?.dshHome, true));
  wrap.appendChild(sec1);

  // Versione disponibile / aggiornamento
  const sec2 = document.createElement("section");
  sec2.className = "section";
  const utitle = document.createElement("h3");
  utitle.textContent = "Versione desiderata";
  sec2.appendChild(utitle);

  if (registryError) {
    const er = document.createElement("div");
    er.className = "warn-box";
    er.textContent = `Registry npm non raggiungibile: ${registryError}`;
    sec2.appendChild(er);
  }

  const selRow = document.createElement("div");
  selRow.className = "form-row";
  const sel = document.createElement("select");
  sel.id = "desiredVersionSelect";
  if (registry) {
    // opzione "latest" (dist-tag)
    const optLatest = document.createElement("option");
    optLatest.value = "latest";
    optLatest.textContent = registry.latest ? `Ultima stabile (${registry.latest})` : "Ultima stabile (—)";
    sel.appendChild(optLatest);
    for (const v of registry.versions.slice(0, 200)) {
      const opt = document.createElement("option");
      opt.value = v;
      const t = tagOf(v, registry);
      opt.textContent = t ? `${v}  (${t})` : v;
      sel.appendChild(opt);
    }
    sel.value = e.settings.desiredVersion ?? "latest";
  } else {
    const opt = document.createElement("option");
    opt.value = "latest";
    opt.textContent = "registry non disponibile";
    sel.appendChild(opt);
  }
  sel.addEventListener("change", () => {
    e.settings.desiredVersion = sel.value === "latest" ? null : sel.value;
    saveSettings();
    renderDetail();
  });

  const selLabel = document.createElement("span");
  selLabel.className = "form-label";
  selLabel.textContent = "Punta a:";
  selRow.appendChild(selLabel);
  selRow.appendChild(sel);
  sec2.appendChild(selRow);

  const targetV = desiredVersionOf(e);
  if (targetV) {
    sec2.appendChild(fmtRow("Ultima stabile (registry)", registry?.latest ?? null, true));
    if (e.probe?.installed && e.probe.version) {
      const cmp = compareVersions(e.probe.version, targetV);
      const upd = cmp < 0 ? `Disponibile: v${targetV} (più recente dell'installata)` : cmp > 0 ? `Installata v${e.probe.version} è più recente di v${targetV}` : `Già alla versione v${e.probe.version}`;
      sec2.appendChild(fmtRow("Aggiornamento", upd));
    }
  }
  wrap.appendChild(sec2);

  // Configurazione runtime
  const sec3 = document.createElement("section");
  sec3.className = "section";
  const ctitle = document.createElement("h3");
  ctitle.textContent = "Configurazione esecuzione";
  sec3.appendChild(ctitle);

  const portRow = document.createElement("div");
  portRow.className = "form-row";
  const portLabel = document.createElement("span");
  portLabel.className = "form-label";
  portLabel.textContent = "Porta (GUI):";
  const portInput = document.createElement("input");
  portInput.type = "number";
  portInput.min = "1024";
  portInput.max = "65535";
  portInput.value = String(e.settings.port);
  portInput.addEventListener("change", () => {
    const n = Number(portInput.value);
    if (Number.isInteger(n) && n > 0 && n < 65536) {
      e.settings.port = n;
      saveSettings();
    } else {
      portInput.value = String(e.settings.port);
    }
  });
  portRow.appendChild(portLabel);
  portRow.appendChild(portInput);
  sec3.appendChild(portRow);

  const wsRow = document.createElement("div");
  wsRow.className = "form-row";
  const wsLabel = document.createElement("span");
  wsLabel.className = "form-label";
  wsLabel.textContent = "Cartella di lavoro:";
  const wsInput = document.createElement("input");
  wsInput.placeholder = e.kind === "windows" ? "es. C:\\Users\\tuo\\workspace (vuoto = home)" : "es. ~/workspace (vuoto = home)";
  wsInput.value = e.settings.workspace ?? "";
  wsInput.addEventListener("change", () => {
    e.settings.workspace = wsInput.value.trim() || null;
    saveSettings();
  });
  wsRow.appendChild(wsLabel);
  wsRow.appendChild(wsInput);
  sec3.appendChild(wsRow);

  const argsRow = document.createElement("div");
  argsRow.className = "form-row";
  const argsLabel = document.createElement("span");
  argsLabel.className = "form-label";
  argsLabel.textContent = "Argomenti extra:";
  const argsInput = document.createElement("input");
  argsInput.placeholder = "es. --host 0.0.0.0 (separati da spazio)";
  argsInput.value = e.settings.extraArgs.join(" ");
  argsInput.addEventListener("change", () => {
    e.settings.extraArgs = argsInput.value.trim().split(/\s+/).filter(Boolean);
    saveSettings();
  });
  argsRow.appendChild(argsLabel);
  argsRow.appendChild(argsInput);
  sec3.appendChild(argsRow);
  wrap.appendChild(sec3);

  // Azioni
  const actions = document.createElement("div");
  actions.className = "actions";
  const mkBtn = (label: string, cls: string, disabled: boolean, fn: () => void): HTMLButtonElement => {
    const b = document.createElement("button");
    b.className = `btn ${cls}`;
    b.textContent = label;
    b.disabled = disabled;
    b.addEventListener("click", fn);
    return b;
  };

  actions.appendChild(
    mkBtn("▶ Avvia", "primary", !e.probe?.installed || e.running || e.busy, () => void actionStart(e)),
  );
  actions.appendChild(
    mkBtn("■ Ferma", "danger", !e.running || e.busy, () => void actionStop(e)),
  );
  actions.appendChild(
    mkBtn("Riautentica tab", "", !e.running || !e.authUrl || e.busy, () => {
      e.authSentAt = null;
      statusMessage = `Riautenticazione di ${e.name}: ricarico la tab con l'URL autenticato…`;
      renderAll();
      void applyLayout();
    }),
  );
  if (!e.probe?.installed) {
    actions.appendChild(
      mkBtn(`Installa dsh (v${targetV ?? "?"})`, "accent", e.busy || !targetV, () => void actionUpdate(e)),
    );
  } else {
    const disabled = e.busy || !targetV || !isUpdateAvailable(e);
    actions.appendChild(
      mkBtn(`Aggiorna a v${targetV ?? "?"}`, "accent", disabled, () => void actionUpdate(e)),
    );
  }
  wrap.appendChild(actions);

  // Messaggi / log
  if (statusMessage || e.note) {
    const logBox = document.createElement("div");
    logBox.className = "log-box";
    if (statusMessage) {
      const msg = document.createElement("div");
      msg.className = "log-msg";
      msg.textContent = statusMessage;
      logBox.appendChild(msg);
    }
    if (e.note) {
      const note = document.createElement("pre");
      note.className = "log-note";
      note.textContent = e.note;
      logBox.appendChild(note);
    }
    wrap.appendChild(logBox);
  }

  if (!e.probe?.installed) {
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent =
      e.kind === "windows"
        ? "dsh non trovato su Windows. Installa con il pulsante qui sopra (richiede bun o npm), oppure manualmente: bun add -g @deepseek-ai/dsh"
        : `dsh non trovato nella distro WSL "${e.name}". Installalo con il pulsante qui sopra (richiede bun o npm dentro la distro), oppure manualmente via terminale WSL.`;
    wrap.appendChild(hint);
  }

  if (e.kind === "wsl" && e.running) {
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent =
      "Nota WSL2: la GUI è raggiungibile da Windows solo se dsh dentro la distro ascolta su 0.0.0.0 oppure con networking WSL in modalità mirror. Se la pagina non si apre, aggiungi --host 0.0.0.0 negli argomenti extra e riavvia.";
    wrap.appendChild(hint);
  }

  box.replaceChildren(wrap);
}

function emptyState(): HTMLDivElement {
  const d = document.createElement("div");
  d.className = "empty";
  d.textContent = "Seleziona un ambiente per i dettagli.";
  return d;
}

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

function buildShell(): void {
  headerBox = document.createElement("div");
  headerBox.id = "headerBox";
  headerBox.className = "header-box";

  const layout = document.createElement("div");
  layout.className = "layout";

  const listBox = document.createElement("div");
  listBox.id = "envListBox";
  listBox.className = "env-list-box";

  const detailBox = document.createElement("div");
  detailBox.id = "detailBox";
  detailBox.className = "detail-box";

  layout.appendChild(listBox);
  layout.appendChild(detailBox);
  appEl.appendChild(headerBox);
  appEl.appendChild(layout);
}

buildShell();

void (async () => {
  await scanEnvironments(true);
  await loadRegistry();
  viewMode = { view: "settings" };
  renderAll();
  void applyLayout();
  // Ridimensionamento finestra -> riposiziona le webview figlie (GUI DSH)
  window.addEventListener("resize", scheduleLayout);
  // Polling di stato "tranquillo": 5s, niente scritture DOM se nulla cambia
  setInterval(() => void refreshRunningStates(), 5000);
})();
