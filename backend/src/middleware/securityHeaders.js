/**
 * Basic security headers for API responses.
 *
 * Intentionally lightweight (no extra dependencies) and safe for JSON APIs.
 * Some headers are only set when the request is served over HTTPS.
 */

export default function securityHeaders(req, res, next) {
  res.setHeader('X-Content-Type-Options', 'nosniff');
  res.setHeader('X-Frame-Options', 'DENY');
  res.setHeader('Referrer-Policy', 'no-referrer');
  res.setHeader('X-DNS-Prefetch-Control', 'off');
  res.setHeader('X-Permitted-Cross-Domain-Policies', 'none');
  res.setHeader('Permissions-Policy', 'geolocation=(), microphone=(), camera=()');
  res.setHeader('Content-Security-Policy', "default-src 'none'; frame-ancestors 'none'; base-uri 'none'");

  const forwardedProto = req.headers['x-forwarded-proto'];
  const isHttps = req.secure || (typeof forwardedProto === 'string' && forwardedProto.includes('https'));
  if (isHttps) {
    res.setHeader('Strict-Transport-Security', 'max-age=15552000; includeSubDomains');
  }

  next();
}

