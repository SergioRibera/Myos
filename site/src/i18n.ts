// Every visible string of the page, in English and Spanish. The name and
// the description come from site.config.json.

export type Lang = 'en' | 'es';

const en = {
  nav: { download: 'Download', github: 'GitHub', language: 'Language' },
  hero: {
    cta: 'Download for {os}',
    ctaGeneric: 'Download',
    ctaChoose: 'Choose your platform',
    meta: '{format}, {arch}, {size}. Version {version}.',
    others: 'Other formats and platforms',
    ios: 'There is no iPhone or iPad download yet.',
  },
  download: {
    title: 'Download',
    version: 'Version {version}, published {date}.',
    loading: 'Looking for the latest version…',
    notes: "What's new",
    detected: 'this device',
    platform: 'Platform',
    platforms: { android: 'Android', windows: 'Windows', macos: 'macOS', linux: 'Linux', ios: 'iPhone/iPad' },
    recommended: 'Recommended for most people',
    recommendedPhone: 'Recommended for most phones',
    architecture: 'Architecture',
    others: 'Other formats for {platform}',
    get: 'Download',
    getFormat: 'Download {format}',
    nix: 'Nix',
    verify: 'Check your download against SHA256SUMS.',
    all: 'All files on GitHub',
    unavailable: 'The release list did not load. Every file is on the GitHub release page.',
    openGithub: 'Open the release page',
    ios: {
      title: 'No build for iPhone or iPad yet',
      body: 'There is no iOS download for now. The project builds for iOS from source: with a Mac you can compile it and install it on your own device.',
      link: 'Source on GitHub',
      need: 'What you need',
      needBody: 'A Mac with Xcode, xcodegen and Rust, and an Apple account to sign the app.',
      steps: 'In Xcode, pick your team under Signing, choose your device and press Run. With a free Apple account the app has to be reinstalled every 7 days.',
    },
    formats: {
      appimage: ['AppImage', 'Runs on any distribution without installing. Download it, allow it to run as a program, open it.'],
      deb: ['.deb', 'Debian, Ubuntu, Mint'],
      rpm: ['.rpm', 'Fedora, openSUSE'],
      flatpak: ['Flatpak', 'Sandboxed, any distribution'],
      'tar.gz': ['.tar.gz', 'Unpack and run'],
      nsis: ['Installer (.exe)', 'The usual installer. Open it and follow the steps.'],
      msi: ['Installer (.msi)', 'For managed installs and IT tools'],
      zip: ['Portable (.zip)', 'Unpack and run, nothing installed'],
      dmg: ['Disk image (.dmg)', 'Open it and drag the app to Applications. Not signed: the first time, right-click the app and choose Open.'],
      pkg: ['Installer (.pkg)', 'Installs the app for every user'],
      app: ['App (.zip)', 'The app itself, zipped'],
      'darwin-tar.gz': ['Archive (.tar.gz)', 'The app in a tarball'],
      apk: ['APK', 'Allow installs from your browser when Android asks.'],
    },
    apk: {
      aarch64: ['APK, arm64', 'Almost every Android phone from recent years. Allow installs from your browser when Android asks.'],
      universal: ['Universal APK', 'Any device, bigger download'],
      x86_64: ['APK, x86_64', 'Emulators and Intel Chromebooks'],
    },
  },
  footer: { source: 'Source', releases: 'All releases' },
  arch: { x86_64: 'x86_64', aarch64: 'aarch64', universal: 'universal' },
};

export type Strings = typeof en;

const es: Strings = {
  nav: { download: 'Descargar', github: 'GitHub', language: 'Idioma' },
  hero: {
    cta: 'Descargar para {os}',
    ctaGeneric: 'Descargar',
    ctaChoose: 'Elige tu plataforma',
    meta: '{format}, {arch}, {size}. Versión {version}.',
    others: 'Otros formatos y plataformas',
    ios: 'Todavía no hay descarga para iPhone ni iPad.',
  },
  download: {
    title: 'Descargar',
    version: 'Versión {version}, publicada el {date}.',
    loading: 'Buscando la última versión…',
    notes: 'Novedades',
    detected: 'este dispositivo',
    platform: 'Plataforma',
    platforms: { android: 'Android', windows: 'Windows', macos: 'macOS', linux: 'Linux', ios: 'iPhone/iPad' },
    recommended: 'Recomendado para la mayoría',
    recommendedPhone: 'Recomendado para la mayoría de teléfonos',
    architecture: 'Arquitectura',
    others: 'Otros formatos para {platform}',
    get: 'Descargar',
    getFormat: 'Descargar {format}',
    nix: 'Nix',
    verify: 'Verifica tu descarga con SHA256SUMS.',
    all: 'Todos los archivos en GitHub',
    unavailable: 'No se pudo cargar la lista de versiones. Todos los archivos están en la página de la versión en GitHub.',
    openGithub: 'Abrir la página de la versión',
    ios: {
      title: 'Todavía no hay versión para iPhone ni iPad',
      body: 'Por ahora no hay descarga para iOS. El proyecto compila para iOS desde el código: con una Mac puedes compilarlo e instalarlo en tu propio dispositivo.',
      link: 'Código en GitHub',
      need: 'Qué necesitas',
      needBody: 'Una Mac con Xcode, xcodegen y Rust, y una cuenta de Apple para firmar la app.',
      steps: 'En Xcode, elige tu equipo en Signing, selecciona tu dispositivo y pulsa Run. Con una cuenta gratuita de Apple hay que reinstalar la app cada 7 días.',
    },
    formats: {
      appimage: ['AppImage', 'Funciona en cualquier distribución sin instalar. Descárgalo, permite ejecutarlo como programa y ábrelo.'],
      deb: ['.deb', 'Debian, Ubuntu, Mint'],
      rpm: ['.rpm', 'Fedora, openSUSE'],
      flatpak: ['Flatpak', 'Aislado, cualquier distribución'],
      'tar.gz': ['.tar.gz', 'Descomprime y ejecuta'],
      nsis: ['Instalador (.exe)', 'El instalador de siempre. Ábrelo y sigue los pasos.'],
      msi: ['Instalador (.msi)', 'Para instalaciones administradas y herramientas de TI'],
      zip: ['Portable (.zip)', 'Descomprime y ejecuta, sin instalar nada'],
      dmg: ['Imagen de disco (.dmg)', 'Ábrela y arrastra la app a Aplicaciones. Sin firmar: la primera vez, haz clic derecho en la app y elige Abrir.'],
      pkg: ['Instalador (.pkg)', 'Instala la app para todos los usuarios'],
      app: ['App (.zip)', 'La app tal cual, comprimida'],
      'darwin-tar.gz': ['Archivo (.tar.gz)', 'La app en un tarball'],
      apk: ['APK', 'Permite instalar desde tu navegador cuando Android lo pida.'],
    },
    apk: {
      aarch64: ['APK, arm64', 'Casi cualquier teléfono Android de los últimos años. Permite instalar desde tu navegador cuando Android lo pida.'],
      universal: ['APK universal', 'Cualquier dispositivo, descarga más grande'],
      x86_64: ['APK, x86_64', 'Emuladores y Chromebooks con Intel'],
    },
  },
  footer: { source: 'Código', releases: 'Todas las versiones' },
  arch: { x86_64: 'x86_64', aarch64: 'aarch64', universal: 'universal' },
};

export const strings: Record<Lang, Strings> = { en, es };

/** Fill `{name}` placeholders. */
export const fill = (text: string, values: Record<string, string>) =>
  text.replace(/\{(\w+)\}/g, (_, k: string) => values[k] ?? `{${k}}`);
