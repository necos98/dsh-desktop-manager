import { version as APP_VERSION } from "../package.json";
import { layoutTabs } from "./api";
import { tagOf } from "./registry";
import "./styles.css";
import {
  TABBAR_H,
  badgeStatus as domainBadgeStatus,
  buildLayoutInput,
  desiredVersionOf as domainDesiredVersionOf,
  envById as domainEnvById,
  showTabEnv,
  toolchainWarning as domainToolchainWarning,
  versionChangeOf as domainVersionChangeOf,
  versionVerb as domainVersionVerb,
  type EnvRow,
  type VersionChange,
  type ViewMode,
} from "./domain/environments";
import { defaultGateway } from "./infra/envGateway";
import {
  checkForManagerUpdate,
  downloadAndInstallUpdate,
  type UpdateInfo,
  type UpdateProgress,
  type UpdaterPhase,
} from "./services/appUpdater";
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
// Pannello ambiente: log + diagnostica WSL (stato UI, mai persistito).
// Log e diagnosi vivono qui (composition root + renderer), le regole di
// lettura restano in EnvironmentService/gateway (DIP). Per riga selezionata:
// - envLog: coda del log (path+tail) oppure errore di lettura;
// - wslDiag: esito passo-passo di diagnose_wsl (solo righe wsl).
// ---------------------------------------------------------------------------

interface EnvLogView {
  loading: boolean;
  path: string | null;
  tail: string | null;
  error: string | null;
}

interface WslDiagView {
  loading: boolean;
  diag: import("./types").WslDiag | null;
  error: string | null;
}

let envLog: EnvLogView = { loading: false, path: null, tail: null, error: null };
let envLogOpen = false;
let wslDiag: WslDiagView = { loading: false, diag: null, error: null };
let logSeq = 0;

// Cache runtime Node per distro (solo memoria, mai persistita): la scelta
// persistita vive in settings.nodeRuntime; qui solo l'elenco rilevato.
interface RuntimesView {
  loading: boolean;
  runtimes: import("./types").NodeRuntime[] | null;
  error: string | null;
}

let runtimesCache: Record<string, RuntimesView> = {};

// ---------------------------------------------------------------------------
// Auto-update del manager (Tauri updater: latest.json delle GitHub Releases).
// ---------------------------------------------------------------------------

let updaterPhase: UpdaterPhase = "idle";
let updaterInfo: UpdateInfo | null = null;
let updaterError: string | null = null;
let updaterProgress: UpdateProgress | null = null;
let updaterAutoChecked = false;

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

/** Direzione del cambio verso la versione desiderata (install/upgrade/
 *  downgrade/reinstall): la UI abilita il pulsante per QUALSIASI direzione. */
function versionChangeOf(e: EnvRow): VersionChange | null {
  return domainVersionChangeOf(e, registry);
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
  if (selectedId !== envId) resetEnvPanels();
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

/** Azzerra i pannelli log/diagnostica al cambio di ambiente (stato per-riga). */
function resetEnvPanels(): void {
  envLog = { loading: false, path: null, tail: null, error: null };
  envLogOpen = false;
  wslDiag = { loading: false, diag: null, error: null };
  logSeq++;
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
  // Blocco preventivo: senza toolchain l'installazione fallirebbe dopo
  // minuti con un errore oscuro — meglio avvisare subito (l'utente la installa).
  const toolchainMsg = domainToolchainWarning(e);
  if (toolchainMsg && !e.probe?.installed) {
    statusMessage = toolchainMsg;
    renderAll();
    return;
  }
  const change = versionChangeOf(e);
  const verb = domainVersionVerb(change);
  const confirmMsg = change === "downgrade"
    ? "Passare dsh su " + e.name + " da v" + (e.probe?.version ?? "?") + " a v" + target + " (downgrade)?"
    : change === "reinstall"
      ? "Reinstallare dsh v" + target + " su " + e.name + "?"
      : null;
  if (confirmMsg && e.running && !window.confirm(confirmMsg + " L'istanza in esecuzione verra fermata e riavviata.")) {
    return;
  }
  if (confirmMsg && !e.running && !window.confirm(confirmMsg)) {
    return;
  }
  e.busy = true;
  statusMessage = `${verb} di dsh su ${e.name} alla versione ${target}… (può richiedere alcuni minuti)`;
  renderAll();
  try {
    const outcome = await envService.updateEnvironment(e, registry);
    if (outcome.ok) {
      statusMessage = outcome.message;
      e.note = outcome.note;
      if (outcome.probe) e.probe = outcome.probe;
      if (e.running) {
        // cambio con istanza attiva: il servizio l'ha riavviata — mostra la tab
        e.busy = false;
        renderAll();
        showEnvTab(e.id);
        return;
      }
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

/** Carica la coda del log dell'ambiente nel pannello (feedback visivo di cosa succede). */
async function actionShowLog(e: EnvRow): Promise<void> {
  const seq = ++logSeq;
  envLogOpen = true;
  envLog = { loading: true, path: null, tail: null, error: null };
  renderDetail();
  const outcome = await envService.readEnvironmentLog(e);
  if (seq !== logSeq) return;
  envLog = outcome.ok
    ? { loading: false, path: outcome.log.path, tail: outcome.log.tail, error: null }
    : { loading: false, path: null, tail: null, error: outcome.error };
  renderDetail();
}

/** Esegue la diagnostica WSL passo-passo e la mostra nel pannello. */
async function actionDiagnose(e: EnvRow): Promise<void> {
  const seq = ++logSeq;
  wslDiag = { loading: true, diag: null, error: null };
  renderDetail();
  const outcome = await envService.diagnoseEnvironment(e);
  if (seq !== logSeq) return;
  wslDiag = outcome.ok
    ? { loading: false, diag: outcome.diag, error: null }
    : { loading: false, diag: null, error: outcome.error };
  renderDetail();
}

// ---------------------------------------------------------------------------
// Auto-update del manager: check manuale + check automatico all'avvio.
// ---------------------------------------------------------------------------

/** Controlla aggiornamenti del manager (pulsante o check automatico). */
async function actionCheckManagerUpdate(auto = false): Promise<void> {
  if (updaterPhase === "checking" || updaterPhase === "downloading" || updaterPhase === "installing") {
    return;
  }
  updaterPhase = "checking";
  updaterError = null;
  updaterProgress = null;
  if (!auto) renderChrome();
  try {
    updaterInfo = await checkForManagerUpdate();
    updaterPhase = updaterInfo ? "available" : "up-to-date";
  } catch (err) {
    updaterPhase = "error";
    updaterError = String(err);
  }
  renderChrome();
  renderDetail();
}

/** Scarica, installa e riavvia il manager sulla nuova versione. */
async function actionInstallManagerUpdate(): Promise<void> {
  if (!updaterInfo || updaterPhase === "downloading" || updaterPhase === "installing") return;
  updaterPhase = "downloading";
  updaterError = null;
  updaterProgress = { downloaded: 0, total: null };
  renderChrome();
  renderDetail();
  try {
    await downloadAndInstallUpdate((p) => {
      updaterProgress = { ...p };
      renderUpdaterProgress();
    });
    updaterPhase = "installing";
    renderChrome();
    renderDetail();
    // Su Windows l'installer NSIS chiude l'app da solo; se siamo ancora qui,
    // l'installazione e' andata a buon fine ma serve un riavvio manuale.
  } catch (err) {
    updaterPhase = "error";
    updaterError = String(err);
    renderChrome();
    renderDetail();
  }
}

/** Aggiorna solo la barra di progresso senza ridisegnare tutto. */
function renderUpdaterProgress(): void {
  const bar = document.getElementById("updaterProgressBar");
  const label = document.getElementById("updaterProgressLabel");
  if (!updaterProgress) return;
  const { downloaded, total } = updaterProgress;
  const mb = (n: number): string => (n / 1024 / 1024).toFixed(1);
  if (bar instanceof HTMLProgressElement && total) {
    bar.max = total;
    bar.value = downloaded;
  }
  if (label) {
    label.textContent = total
      ? `${mb(downloaded)} / ${mb(total)} MB`
      : `Scaricati ${mb(downloaded)} MB…`;
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
  const versionEl = document.createElement("span");
  versionEl.className = "app-version" + (updaterPhase === "available" ? " has-update" : "");
  versionEl.id = "appVersion";
  versionEl.textContent = "v" + APP_VERSION;
  versionEl.title = updaterVersionTitle();
  if (updaterPhase === "available") {
    versionEl.addEventListener("click", () => showSettingsTab());
  }
  right.appendChild(versionEl);
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

/** Tooltip dell'etichetta versione in base alla fase dell'updater. */
function updaterVersionTitle(): string {
  switch (updaterPhase) {
    case "checking":
      return "Controllo aggiornamenti in corso...";
    case "available":
      return updaterInfo
        ? "Aggiornamento manager disponibile: v" + updaterInfo.version + " - apri le Impostazioni"
        : "Aggiornamento disponibile";
    case "up-to-date":
      return "Manager aggiornato - ricontrolla";
    case "downloading":
      return "Download aggiornamento in corso...";
    case "installing":
      return "Installazione aggiornamento...";
    case "error":
      return updaterError ? "Errore aggiornamento: " + updaterError : "Errore controllo aggiornamenti";
    default:
      return "DSH Manager v" + APP_VERSION + " — vedi la sezione Aggiornamento manager";
  }
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
      if (selectedId !== e.id) resetEnvPanels();
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

/** Sezione "Aggiornamento manager": check/download/install dall'app. */
function renderManagerUpdateSection(): HTMLElement {
  const sec = document.createElement("section");
  sec.className = "section" + (updaterPhase === "available" ? " update-available" : "");

  const title = document.createElement("h3");
  title.textContent = "Aggiornamento manager";
  sec.appendChild(title);

  const row = document.createElement("div");
  row.className = "form-row";

  const status = document.createElement("span");
  status.className = "form-label update-status";
  status.textContent = updaterStatusText();
  row.appendChild(status);

  const checkBtn = document.createElement("button");
  checkBtn.className = "btn";
  checkBtn.textContent =
    updaterPhase === "checking" ? "Controllo..." : "Controlla aggiornamenti";
  checkBtn.disabled =
    updaterPhase === "checking" ||
    updaterPhase === "downloading" ||
    updaterPhase === "installing";
  checkBtn.addEventListener("click", () => void actionCheckManagerUpdate(false));
  row.appendChild(checkBtn);

  if (updaterPhase === "available" && updaterInfo) {
    const installBtn = document.createElement("button");
    installBtn.className = "btn accent";
    installBtn.textContent = "Scarica e installa v" + updaterInfo.version;
    installBtn.addEventListener("click", () => void actionInstallManagerUpdate());
    row.appendChild(installBtn);
  }
  sec.appendChild(row);

  if (updaterInfo && (updaterPhase === "available" || updaterPhase === "downloading" || updaterPhase === "installing")) {
    if (updaterInfo.body) {
      const notes = document.createElement("pre");
      notes.className = "log-note";
      notes.textContent = updaterInfo.body.slice(0, 1500);
      sec.appendChild(notes);
    }
  }

  if (updaterPhase === "downloading" && updaterProgress) {
    const pRow = document.createElement("div");
    pRow.className = "form-row";
    const bar = document.createElement("progress");
    bar.id = "updaterProgressBar";
    bar.className = "update-progress";
    if (updaterProgress.total) {
      bar.max = updaterProgress.total;
      bar.value = updaterProgress.downloaded;
    }
    const label = document.createElement("span");
    label.id = "updaterProgressLabel";
    label.className = "info-value";
    pRow.appendChild(bar);
    pRow.appendChild(label);
    sec.appendChild(pRow);
    // Prima pittura del testo (il resto arriva via renderUpdaterProgress)
    queueMicrotask(() => renderUpdaterProgress());
  }

  if (updaterPhase === "installing") {
    const msg = document.createElement("div");
    msg.className = "hint";
    msg.textContent =
      "Installazione in corso: l'app si chiudera' e si riaprira' aggiornata. Se resta aperta, riavviala manualmente.";
    sec.appendChild(msg);
  }

  if (updaterPhase === "error" && updaterError) {
    const er = document.createElement("div");
    er.className = "warn-box";
    er.textContent = "Controllo aggiornamenti fallito: " + updaterError;
    sec.appendChild(er);
  }

  return sec;
}

/** Testo di stato compatto per la sezione updater. */
function updaterStatusText(): string {
  switch (updaterPhase) {
    case "checking":
      return "Controllo in corso...";
    case "available":
      return updaterInfo ? "Disponibile: v" + updaterInfo.version : "Aggiornamento disponibile";
    case "up-to-date":
      return "Sei aggiornato";
    case "downloading":
      return "Download in corso...";
    case "installing":
      return "Installazione...";
    case "error":
      return "Errore controllo";
    default:
      return "Mai controllato";
  }
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

  wrap.appendChild(renderManagerUpdateSection());

  // Rilevamento
  const sec1 = document.createElement("section");
  sec1.className = "section";
  const stitle = document.createElement("h3");
  stitle.textContent = "Rilevamento dsh";
  sec1.appendChild(stitle);
  const p = e.probe;
  if (p?.error && (!p.installed || !p.version)) {
    const er = document.createElement("div");
    er.className = "warn-box";
    er.textContent =
      p.installed && !p.version
        ? `dsh rilevato ma non funzionante: ${p.error}`
        : `Errore rilevamento: ${p.error}`;
    sec1.appendChild(er);
  }
  sec1.appendChild(fmtRow("Installato", p?.installed ? "Sì" : "No"));
  sec1.appendChild(fmtRow("Versione installata", p?.installed ? p?.version : null, true));
  sec1.appendChild(fmtRow("Eseguibile", p?.executable, true));
  sec1.appendChild(fmtRow("DSH_HOME", p?.dshHome, true));
  // Toolchain native (nelle distro WSL l'interop /mnt/* e ignorata dal
  // backend): l'utente deve installarle da solo — qui solo avviso.
  const toolchainLine =
    p?.hasBun === null || p?.hasBun === undefined
      ? "—"
      : `${p.hasBun ? "bun ✓" : "bun ✗"} · ${p.hasNpm ? "npm ✓" : "npm ✗"}${e.kind === "wsl" ? " (nativi)" : ""}`;
  sec1.appendChild(fmtRow("Toolchain (bun/npm)", p ? toolchainLine : null, true));
  const toolchainMsg = domainToolchainWarning(e);
  if (toolchainMsg) {
    const warn = document.createElement("div");
    warn.className = "warn-box";
    warn.textContent = toolchainMsg;
    sec1.appendChild(warn);
  }
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
      const change = versionChangeOf(e);
      const upd = change === "upgrade"
        ? `Disponibile: v${targetV} (piu recente dell'installata)`
        : change === "downgrade"
          ? `Downgrade: v${targetV} (precedente all'installata v${e.probe.version})`
          : change === "reinstall"
            ? `Gia alla versione v${e.probe.version} (reinstallabile)`
            : `Gia alla versione v${e.probe.version}`;
      sec2.appendChild(fmtRow("Versione", upd));
    } else if (registryError) {
      sec2.appendChild(fmtRow("Versione", "registry non raggiungibile: impossibile scegliere la versione"));
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
  argsInput.placeholder = e.kind === "wsl"
    ? "es. --host 0.0.0.0 (solo flag semplici, niente simboli shell)"
    : "es. --host 0.0.0.0 (separati da spazio)";
  argsInput.value = e.settings.extraArgs.join(" ");
  argsInput.addEventListener("change", () => {
    e.settings.extraArgs = argsInput.value.trim().split(/\s+/).filter(Boolean);
    saveSettings();
  });
  argsRow.appendChild(argsLabel);
  argsRow.appendChild(argsInput);
  sec3.appendChild(argsRow);
  // Runtime Node (solo WSL): scelta utente persistita, mai lotteria.
  if (e.kind === "wsl") {
    sec3.appendChild(renderNodeRuntimeRow(e));
  }
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
    mkBtn(envLogOpen ? "↻ Ricarica log" : "Mostra log", "", e.busy || envLog.loading, () => void actionShowLog(e)),
  );
  if (e.kind === "wsl") {
    actions.appendChild(
      mkBtn("🩺 Diagnostica WSL", "", e.busy || wslDiag.loading, () => void actionDiagnose(e)),
    );
  }
  actions.appendChild(
    mkBtn("Riautentica tab", "", !e.running || !e.authUrl || e.busy, () => {
      e.authSentAt = null;
      statusMessage = `Riautenticazione di ${e.name}: ricarico la tab con l'URL autenticato…`;
      renderAll();
      void applyLayout();
    }),
  );
  // Cambio versione libero: install/upgrade/downgrade/reinstall condividono
  // lo stesso pulsante (il backend sovrascrive la versione pinnata in ogni
  // caso). Etichetta dalla direzione reale, mai disabilitato per downgrade.
  if (!e.probe?.installed) {
    actions.appendChild(
      mkBtn(`Installa dsh (v${targetV ?? "?"})`, "accent", e.busy || !targetV, () => void actionUpdate(e)),
    );
  } else {
    const change = versionChangeOf(e);
    const label = change === "downgrade"
      ? `Downgrade a v${targetV ?? "?"}`
      : change === "reinstall"
        ? `Reinstalla v${targetV ?? "?"}`
        : `Aggiorna a v${targetV ?? "?"}`;
    actions.appendChild(
      mkBtn(label, "accent", e.busy || !targetV, () => void actionUpdate(e)),
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
        ? "dsh non trovato su Windows. Installa con il pulsante qui sopra (richiede bun o npm installati da te), oppure manualmente: bun add -g @deepseek-ai/dsh"
        : `dsh nativo non trovato nella distro WSL "${e.name}" (eventuali copie Windows via interop vengono ignorate). Installalo con il pulsante qui sopra (richiede bun o npm nativi installati da te dentro la distro), oppure manualmente via terminale WSL.`;
    wrap.appendChild(hint);
  }

  wrap.appendChild(renderEnvLogSection(e));
  if (e.kind === "wsl") {
    wrap.appendChild(renderWslDiagSection(e));
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

/** Pannello "Log ambiente": coda del log con percorso, errore o spinner. */
function renderEnvLogSection(e: EnvRow): HTMLElement {
  const sec = document.createElement("section");
  sec.className = "section";
  const title = document.createElement("h3");
  title.textContent = "Log ambiente";
  sec.appendChild(title);
  if (!envLogOpen && !envLog.loading && !envLog.tail && !envLog.error) {
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent =
      "Qui vedi cosa sta facendo dsh: premi «Mostra log» per leggere la coda del log " +
      (e.kind === "windows" ? "Windows." : `della distro "${e.name}".`);
    sec.appendChild(hint);
    return sec;
  }
  if (envLog.loading) {
    const loading = document.createElement("div");
    loading.className = "hint";
    loading.textContent = "Lettura log in corso…";
    sec.appendChild(loading);
    return sec;
  }
  if (envLog.error) {
    const er = document.createElement("div");
    er.className = "warn-box";
    er.textContent = envLog.error;
    sec.appendChild(er);
    return sec;
  }
  if (envLog.path) sec.appendChild(fmtRow("File", envLog.path, true));
  const pre = document.createElement("pre");
  pre.className = "log-note";
  pre.textContent = envLog.tail ?? "(log vuoto)";
  sec.appendChild(pre);
  return sec;
}

/** Pannello "Diagnostica WSL": checklist passo-passo (stato, dsh, toolchain, porte, log). */
function renderWslDiagSection(e: EnvRow): HTMLElement {
  const sec = document.createElement("section");
  sec.className = "section";
  const title = document.createElement("h3");
  title.textContent = "Diagnostica WSL";
  sec.appendChild(title);
  if (!wslDiag.loading && !wslDiag.diag && !wslDiag.error) {
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent =
      "Se l'avvio fallisce, premi «🩺 Diagnostica WSL»: controlla in sequenza distro, dsh nativo, toolchain native bun/npm, porte e log (l'interop Windows /mnt/* viene ignorata).";
    sec.appendChild(hint);
    return sec;
  }
  if (wslDiag.loading) {
    const loading = document.createElement("div");
    loading.className = "hint";
    loading.textContent = `Diagnostica della distro "${e.distro ?? e.name}" in corso (puo richiedere fino a 2 minuti al primo avvio)…`;
    sec.appendChild(loading);
    return sec;
  }
  if (wslDiag.error) {
    const er = document.createElement("div");
    er.className = "warn-box";
    er.textContent = wslDiag.error;
    sec.appendChild(er);
    return sec;
  }
  const d = wslDiag.diag;
  if (!d) return sec;
  const mark = (ok: boolean | null | undefined): string => (ok === true ? "✓" : ok === false ? "✗" : "?");
  sec.appendChild(fmtRow("Distro (stato)", `${d.distro} (${d.state ?? "sconosciuto"})`, true));
  sec.appendChild(fmtRow("dsh", d.dshInstalled ? `installato${d.dshVersion ? ` (v${d.dshVersion})` : ""}` : "non trovato", true));
  sec.appendChild(fmtRow("Toolchain nativa", `bun ${mark(d.hasBun)} · npm ${mark(d.hasNpm)}`, true));
  const portLine =
    `nella distro: ${d.portOpenInDistro === true ? "aperta ✓" : d.portOpenInDistro === false ? "chiusa ✗" : "non verificata ?"} · ` +
    `da Windows: ${d.portOpenFromWindows ? "aperta ✓" : "chiusa ✗"} (porta ${e.settings.port})`;
  sec.appendChild(fmtRow("Porta", portLine));
  if (!d.hasBun && !d.hasNpm) {
    const warn = document.createElement("div");
    warn.className = "warn-box";
    warn.textContent =
      `Nella distro "${d.distro}" mancano sia bun che npm nativi (eventuali copie Windows via interop vengono ignorate): installa prima una toolchain nativa ` +
      `(es. \`curl -fsSL https://bun.sh/install | bash\` oppure \`sudo apt install nodejs npm\`), poi installa dsh. ` +
      `Il manager non installa toolchain da solo.`;
    sec.appendChild(warn);
  }
  if (d.portOpenInDistro === true && !d.portOpenFromWindows) {
    const warn = document.createElement("div");
    warn.className = "warn-box";
    warn.textContent =
      "Il server risponde DENTRO la distro ma non da Windows: tipico WSL2 in modalita NAT. " +
      "Aggiungi --host 0.0.0.0 negli argomenti extra e riavvia.";
    sec.appendChild(warn);
  }
  if (d.error) {
    const er = document.createElement("div");
    er.className = "warn-box";
    er.textContent = d.error;
    sec.appendChild(er);
  }
  if (d.logTail) {
    const pre = document.createElement("pre");
    pre.className = "log-note";
    pre.textContent = d.logTail;
    sec.appendChild(pre);
  }
  return sec;
}

/** Riga "Runtime Node" (solo WSL): select tra Automatico e versioni rilevate.
 *  La scelta si salva in settings.nodeRuntime e si riusa in probe/start/
 *  update/diag (stesso mondo ovunque). Scelta sparita -> errore esplicito
 *  dal backend al prossimo avvio (mai fallback silenzioso). */
function renderNodeRuntimeRow(e: EnvRow): HTMLElement {
  const row = document.createElement("div");
  row.className = "form-row";
  const label = document.createElement("span");
  label.className = "form-label";
  label.textContent = "Runtime Node:";
  row.appendChild(label);
  const sel = document.createElement("select");
  sel.id = "nodeRuntimeSelect";
  const cached = runtimesCache[e.id];
  const optAuto = document.createElement("option");
  optAuto.value = "";
  optAuto.textContent = "Automatico (piu recente)";
  sel.appendChild(optAuto);
  if (cached?.runtimes) {
    for (const r of cached.runtimes) {
      const opt = document.createElement("option");
      opt.value = r.id;
      opt.textContent = r.label + (r.nodeVersion ? ` — ${r.nodeVersion}` : "");
      sel.appendChild(opt);
    }
    // Scelta salvata ma non piu rilevata: voce esplicita (non sparisce).
    if (e.settings.nodeRuntime && !cached.runtimes.some((r) => r.id === e.settings.nodeRuntime)) {
      const opt = document.createElement("option");
      opt.value = e.settings.nodeRuntime;
      opt.textContent = `Non disponibile: ${e.settings.nodeRuntime} (riseleziona)`;
      sel.appendChild(opt);
    }
    sel.value = e.settings.nodeRuntime ?? "";
  } else if (cached?.loading) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = "Rilevamento runtime…";
    sel.appendChild(opt);
  } else {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = cached?.error ? "Errore elenco (riprova)" : "Carica…";
    sel.appendChild(opt);
    // Carica una volta sola per riga (cache per id ambiente).
    if (!cached) {
      runtimesCache[e.id] = { loading: true, runtimes: null, error: null };
      void envService.listRuntimes(e).then((out) => {
        runtimesCache[e.id] = out.ok
          ? { loading: false, runtimes: out.runtimes, error: null }
          : { loading: false, runtimes: null, error: out.error };
        // Ridisegna solo se la riga e ancora selezionata.
        if (selectedId === e.id) renderDetail();
      });
    }
  }
  sel.addEventListener("change", () => {
    e.settings.nodeRuntime = sel.value || null;
    saveSettings();
    statusMessage = sel.value
      ? `Runtime Node impostato su ${sel.selectedOptions[0]?.textContent ?? sel.value}: la prossima scansione usera quello.`
      : "Runtime Node su Automatico: verra usata la versione piu recente.";
    renderAll();
    // Re-probe immediata col runtime scelto (feedback subito, non al giro dopo).
    void (async () => {
      const rt = e.settings.nodeRuntime ?? null;
      try {
        const { defaultGateway } = await import("./infra/envGateway");
        e.probe = await defaultGateway.probeWsl(e.distro ?? "", rt);
      } catch (err) {
        statusMessage = `Re-probe fallita: ${String(err)}`;
      }
      renderAll();
    })();
  });
  row.appendChild(sel);
  return row;
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
  // Check aggiornamenti manager all'avvio (silenzioso, una sola volta)
  if (!updaterAutoChecked) {
    updaterAutoChecked = true;
    setTimeout(() => void actionCheckManagerUpdate(true), 3000);
  }
})();
