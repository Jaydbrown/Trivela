/**
 * Optional API key authentication middleware.
 *
 * If the provided API key is empty, the middleware is a no-op to keep local
 * development convenient.
 */

function parseApiKeys(value) {
  if (!value) {
    return [];
  }

  if (Array.isArray(value)) {
    return value.map((key) => String(key).trim()).filter(Boolean);
  }

  return String(value)
    .split(',')
    .map((key) => key.trim())
    .filter(Boolean);
}

function normalizeApiKeys(value) {
  const keys = parseApiKeys(value);
  return [...new Set(keys)];
}

function readProvidedKey(req) {
  const headerKey = req.headers['x-api-key'];
  if (typeof headerKey === 'string' && headerKey.trim()) {
    return headerKey.trim();
  }

  const authorization = req.headers.authorization;
  if (typeof authorization === 'string' && authorization.trim()) {
    const match = authorization.match(/^\s*Bearer\s+(.+)\s*$/i);
    if (match?.[1]) {
      return match[1].trim();
    }
  }

  const queryKey = req.query?.api_key;
  if (typeof queryKey === 'string' && queryKey.trim()) {
    return queryKey.trim();
  }

  return '';
}

export default function createApiKeyAuth({
  apiKeys = process.env.TRIVELA_API_KEYS || process.env.TRIVELA_API_KEY || '',
} = {}) {
  const allowedKeys = normalizeApiKeys(apiKeys);
  const allowedKeySet = new Set(allowedKeys);

  return function requireApiKey(req, res, next) {
    if (allowedKeySet.size === 0) {
      return next();
    }

    const provided = readProvidedKey(req);

    if (provided && allowedKeySet.has(provided)) {
      return next();
    }

    return res.status(401).json({ error: 'Unauthorized – valid API key required.' });
  };
}
