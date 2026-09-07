// Facciata compatibile: il contratto storico di registry.ts resta valido,
// ma la logica vive nei nuovi moduli (domain/semver + infra/registryClient).
export { compareVersions, compareVersionsDesc } from './domain/semver';
export { NpmRegistryClient, fetchRegistry, tagOf } from './infra/registryClient';