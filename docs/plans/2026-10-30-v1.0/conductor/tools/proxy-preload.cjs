// Conductor-only workaround (environment): /root/gc-tools's @vercel/sandbox CJS build uses undici.Agent, which ignores
// HTTPS_PROXY; its ESM build was patched to EnvHttpProxyAgent. Make the CJS one match. Not in the repo.
const u = require('/root/gc-tools/node_modules/undici');
u.Agent = u.EnvHttpProxyAgent;
