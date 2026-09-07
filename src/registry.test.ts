import { describe, expect, it, vi, afterEach } from "vitest";
import { compareVersions, compareVersionsDesc, fetchRegistry, tagOf } from "./registry";
import type { RegistryData } from "./types";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("compareVersions", () => {
  it("ordina major/minor/patch", () => {
    expect(compareVersions("1.0.0", "2.0.0")).toBeLessThan(0);
    expect(compareVersions("0.1.0", "0.2.0")).toBeLessThan(0);
    expect(compareVersions("0.0.1", "0.0.2")).toBeLessThan(0);
    expect(compareVersions("1.2.3", "1.2.3")).toBe(0);
    expect(compareVersions("2.0.0", "1.9.9")).toBeGreaterThan(0);
  });

  it("la release viene dopo la pre-release", () => {
    expect(compareVersions("1.0.0-alpha", "1.0.0")).toBeLessThan(0);
    expect(compareVersions("1.0.0", "1.0.0-alpha")).toBeGreaterThan(0);
  });

  it("ordina le pre-release numeriche", () => {
    expect(compareVersions("1.0.0-alpha.1", "1.0.0-alpha.2")).toBeLessThan(0);
    expect(compareVersions("1.0.0-beta", "1.0.0-alpha")).toBeGreaterThan(0);
  });

  it("gestisce versioni non semver con fallback lessicografico", () => {
    expect(compareVersions("foo", "foo")).toBe(0);
    expect(typeof compareVersions("foo", "bar")).toBe("number");
  });
});

describe("compareVersionsDesc", () => {
  it("ordina in modo decrescente", () => {
    const versions = ["1.0.0", "2.0.0", "1.5.0", "0.9.9"];
    expect([...versions].sort(compareVersionsDesc)).toEqual([
      "2.0.0",
      "1.5.0",
      "1.0.0",
      "0.9.9",
    ]);
  });
});

describe("tagOf", () => {
  const data: RegistryData = {
    latest: "2.0.0",
    distTags: { latest: "2.0.0", beta: "2.1.0-beta.1" },
    versions: ["2.1.0-beta.1", "2.0.0", "1.0.0"],
  };

  it("ritorna latest quando presente", () => {
    expect(tagOf("2.0.0", data)).toBe("latest");
  });

  it("ritorna il tag corrispondente", () => {
    expect(tagOf("2.1.0-beta.1", data)).toBe("beta");
  });

  it("ritorna null se nessun tag corrisponde", () => {
    expect(tagOf("1.0.0", data)).toBeNull();
  });
});

describe("fetchRegistry", () => {
  it("parsa dist-tags e ordina le versioni in modo decrescente", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        json: async () => ({
          "dist-tags": { latest: "2.0.0" },
          versions: { "1.0.0": {}, "2.0.0": {}, "1.5.0": {} },
        }),
      })),
    );
    const data = await fetchRegistry();
    expect(data.latest).toBe("2.0.0");
    expect(data.distTags).toEqual({ latest: "2.0.0" });
    expect(data.versions).toEqual(["2.0.0", "1.5.0", "1.0.0"]);
  });

  it("lancia un errore su risposta HTTP non-ok", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({ ok: false, status: 500, json: async () => ({}) })),
    );
    await expect(fetchRegistry()).rejects.toThrow("Registry npm: HTTP 500");
  });

  it("gestisce payload senza versions", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        ok: true,
        json: async () => ({}),
      })),
    );
    const data = await fetchRegistry();
    expect(data.latest).toBeNull();
    expect(data.versions).toEqual([]);
  });
});
