import { describe, expect, it } from 'vitest';
import { renderLogBox } from './logBox';

describe('renderLogBox', () => {
  it('null quando non c e nulla da mostrare', () => {
    expect(renderLogBox('', '')).toBeNull();
  });

  it('mostra messaggio e output integrale in monospace', () => {
    const box = renderLogBox('Aggiornamento fallito (exit -1).', 'riga1\nriga2');
    expect(box).not.toBeNull();
    expect(box?.querySelector('.log-msg')?.textContent).toBe('Aggiornamento fallito (exit -1).');
    expect(box?.querySelector('pre.log-note')?.textContent).toBe('riga1\nriga2');
  });

  it('senza output non aggiunge il pulsante Copia', () => {
    const box = renderLogBox('solo messaggio', '');
    expect(box?.querySelector('button')).toBeNull();
    expect(box?.querySelector('pre.log-note')).toBeNull();
  });

  it('Copia scrive negli appunti l output mostrato', async () => {
    const written: string[] = [];
    const box = renderLogBox('msg', 'errore esecuzione npm: file non trovato', async (t) => {
      written.push(t);
    });
    const button = box?.querySelector('button');
    expect(button?.textContent).toBe('Copia');
    button?.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(written).toEqual(['errore esecuzione npm: file non trovato']);
    expect(button?.textContent).toBe('Copiato ✓');
  });

  it('copia fallita -> etichetta di errore sul pulsante', async () => {
    const box = renderLogBox('msg', 'output', async () => {
      throw new Error('niente appunti');
    });
    const button = box?.querySelector('button');
    button?.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(button?.textContent).toBe('Copia fallita');
  });
});
