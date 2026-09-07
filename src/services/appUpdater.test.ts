import { describe, expect, it, vi } from "vitest";
import {
  __setUpdateCheckForTests,
  checkForManagerUpdate,
  downloadAndInstallUpdate,
} from "./appUpdater";

describe("appUpdater", () => {
  it("ritorna null quando non ci sono aggiornamenti", async () => {
    __setUpdateCheckForTests(async () => null);
    try {
      await expect(checkForManagerUpdate()).resolves.toBeNull();
    } finally {
      __setUpdateCheckForTests(null);
    }
  });

  it("mappa i metadati dell'update e chiude la risorsa", async () => {
    const close = vi.fn(async () => undefined);
    __setUpdateCheckForTests(async () => ({
      version: "0.3.0",
      currentVersion: "0.2.1",
      body: "Novita'",
      date: "2026-09-01T00:00:00Z",
      downloadAndInstall: async () => undefined,
      close,
    }));
    try {
      const info = await checkForManagerUpdate();
      expect(info).toEqual({
        version: "0.3.0",
        currentVersion: "0.2.1",
        body: "Novita'",
        date: "2026-09-01T00:00:00Z",
      });
      expect(close).toHaveBeenCalledOnce();
    } finally {
      __setUpdateCheckForTests(null);
    }
  });

  it("downloadAndInstallUpdate riporta Started/Progress/Finished", async () => {
    const seen: string[] = [];
    __setUpdateCheckForTests(async () => ({
      version: "0.3.0",
      currentVersion: "0.2.1",
      downloadAndInstall: async (onEvent) => {
        onEvent?.({ event: "Started", data: { contentLength: 100 } });
        seen.push("started");
        onEvent?.({ event: "Progress", data: { chunkLength: 40 } });
        onEvent?.({ event: "Progress", data: { chunkLength: 60 } });
        onEvent?.({ event: "Finished", data: {} });
      },
      close: async () => undefined,
    }));
    const progress: { downloaded: number; total: number | null }[] = [];
    try {
      await downloadAndInstallUpdate((p) => progress.push({ ...p }));
      expect(seen).toEqual(["started"]);
      expect(progress[0]).toEqual({ downloaded: 0, total: 100 });
      expect(progress.at(-1)).toEqual({ downloaded: 100, total: 100 });
    } finally {
      __setUpdateCheckForTests(null);
    }
  });

  it("propaga gli errori di rete del check", async () => {
    __setUpdateCheckForTests(async () => {
      throw new Error("offline");
    });
    try {
      await expect(checkForManagerUpdate()).rejects.toThrow("offline");
    } finally {
      __setUpdateCheckForTests(null);
    }
  });
});
