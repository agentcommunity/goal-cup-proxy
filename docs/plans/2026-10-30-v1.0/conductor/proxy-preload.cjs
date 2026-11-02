// Environment workaround: the sandbox SDK's CommonJS build uses undici.Agent, which ignores HTTPS_PROXY.
// Make it use EnvHttpProxyAgent, as patch-sandbox.sh does for the ESM build.
// Callers set NODE_OPTIONS="--use-openssl-ca --no-warnings --require $GC_TOOLS/proxy-preload.cjs".
const u = require(require('path').join(__dirname, 'node_modules', 'undici'));
u.Agent = u.EnvHttpProxyAgent;
