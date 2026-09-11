import type { NextConfig } from "next";
const config: NextConfig = {
  poweredByHeader: false,
  productionBrowserSourceMaps: false,
  serverExternalPackages: ["pg"],
  async headers() {
    return [{ source: "/:path*", headers: [
      {key:"Cache-Control",value:"private, no-store"},
      {key:"X-Content-Type-Options",value:"nosniff"},
      // Preserve same-origin POST Origin; no-referrer makes Chrome send null.
      {key:"Referrer-Policy",value:"same-origin"},
      {key:"X-Frame-Options",value:"DENY"},
      {key:"Permissions-Policy",value:"camera=(), microphone=(), geolocation=()"},
      {key:"Content-Security-Policy",value:"default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"}
    ]}];
  }
};
export default config;
