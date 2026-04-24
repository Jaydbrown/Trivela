/**
 * Optional API key authentication middleware.
 *
 * If the provided API key is empty, the middleware is a no-op to keep local
 * development convenient.
 */

export default function createApiKeyAuth({
  apiKey = process.env.TRIVELA_API_KEY || '',
} = {}) {
  return function requireApiKey(req, res, next) {
    if (!apiKey) {
      return next();
    }

    const provided = req.headers['x-api-key'] || req.query.api_key;

    if (provided === apiKey) {
      req.auth = {
        type: 'apiKey',
        apiKey: String(provided),
      };
      return next();
    }

    return res.status(401).json({ error: 'Unauthorized – valid API key required.' });
  };
}
