// @ts-check
import { defineConfig } from 'astro/config';
import config from './site.config.json' with { type: 'json' };

// `base` other than "/" serves the site under a path of a domain that
// hosts other things too (example.com/myapp, for example): the
// build goes to dist/<base> and wrangler.jsonc routes `<domain>/<base>*`.
const base = config.base.replace(/\/?$/, '/');

export default defineConfig({
  site: config.site,
  base,
  outDir: base === '/' ? './dist' : `./dist${base}`,
  trailingSlash: 'always',
  i18n: {
    locales: ['en', 'es'],
    defaultLocale: 'en',
    routing: { prefixDefaultLocale: false },
  },
});
