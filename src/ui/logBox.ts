// Presentazione: blocco "messaggio + output" (monospace) con pulsante «Copia».
// Separato da main.ts (che resta composition root della UI) e senza stato:
// riceve solo il testo da mostrare, cosi e testabile in jsdom.

/** Scrittura negli appunti, iniettabile nei test (default: API clipboard). */
export type ClipboardWriter = (text: string) => Promise<void>;

/** Scrittura negli appunti di produzione: API clipboard, con fallback
 *  textarea+execCommand per le webview dove `navigator.clipboard` manca. */
export const clipboardWriter: ClipboardWriter = async (text: string) => {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  const ok = document.execCommand("copy");
  document.body.removeChild(area);
  if (!ok) throw new Error("copia non disponibile");
};

/**
 * Blocco log: riga di messaggio + output completo in `<pre>` monospace con
 * pulsante «Copia». Ritorna null se non c'e nulla da mostrare (la UI non
 * aggiunge il contenitore vuoto).
 */
export function renderLogBox(message: string, note: string, copy: ClipboardWriter = clipboardWriter): HTMLElement | null {
  if (!message && !note) return null;
  const box = document.createElement("div");
  box.className = "log-box";
  if (message) {
    const msg = document.createElement("div");
    msg.className = "log-msg";
    msg.textContent = message;
    box.appendChild(msg);
  }
  if (note) {
    const head = document.createElement("div");
    head.className = "log-note-head";
    const button = document.createElement("button");
    button.className = "btn";
    button.textContent = "Copia";
    button.addEventListener("click", () => {
      void copy(note).then(
        () => {
          button.textContent = "Copiato ✓";
        },
        () => {
          button.textContent = "Copia fallita";
        },
      );
    });
    head.appendChild(button);
    box.appendChild(head);
    const pre = document.createElement("pre");
    pre.className = "log-note";
    pre.textContent = note;
    box.appendChild(pre);
  }
  return box;
}
