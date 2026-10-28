pragma Singleton
import QtQuick

// SYNTHETIC DESIGN DATA, taken from the Archie Mac mockups. Nothing here comes from archied,
// Sentry or the filesystem; every screen that uses it says "Diseño" (the DATOS DE DISEÑO notice
// and per-section chips). The bridge-fed parts (link, control center, activity jobs, Sentry)
// do not read from here.
QtObject {
    readonly property var dock: [
        { id: "orb",    name: "Orb",     icon: "dkOrb",    running: true  },
        { id: "world",  name: "World",   icon: "dkWorld",  running: false },
        { id: "vault",  name: "Vault",   icon: "dkVault",  running: true  },
        { id: "forge",  name: "Forge",   icon: "dkForge",  running: false },
        { id: "sentry", name: "Sentry",  icon: "dkSentry", running: false },
        { id: "world",  name: "World",   icon: "dkWorld",  running: false },
        { id: "stage",  name: "Stage",   icon: "dkStage",  running: true  },
        { id: "set",    name: "Ajustes", icon: "dkSet",    running: false }
    ]
    readonly property var tabs: ["Entregas", "Workspaces"]
    readonly property var crumbs: ["Inicio", "Archie", "motor-bracket"]
    readonly property var places: [
        { name: "Entregas",   icon: "deliveries", domain: "" },
        { name: "Workspaces", icon: "workspaces", domain: "" },
        { name: "Documentos", icon: "documents",  domain: "personal" },
        { name: "Trabajo",    icon: "work",       domain: "work" },
        { name: "Recientes",  icon: "recents",    domain: "" },
        { name: "Papelera",   icon: "trash",      domain: "" }
    ]
    // icon tint: folders t2, otherwise the file category colour.
    readonly property var files: [
        { name: "motor-bracket",     icon: "folder", cat: "",          size: "—",      kind: "carpeta", when: "hoy 14:21",    domain: "",         archie: "" },
        { name: "soporte-v3.step",   icon: "cad",    cat: "other",     size: "2,4 MB", kind: "",        when: "hoy 14:18",    domain: "",         archie: "14:18" },
        { name: "informe-q3.docx",   icon: "doc",    cat: "documents", size: "148 KB", kind: "",        when: "ayer 18:40",   domain: "work",     archie: "" },
        { name: "render-final.mp4",  icon: "video",  cat: "media",     size: "86 MB",  kind: "",        when: "10 oct 09:12", domain: "",         archie: "", loading: true },
        { name: "fotos-viaje",       icon: "folder", cat: "",          size: "—",      kind: "carpeta", when: "3 oct 21:05",  domain: "personal", archie: "" },
        { name: "presupuesto.xlsx",  icon: "sheet",  cat: "data",      size: "62 KB",  kind: "",        when: "8 oct 11:30",  domain: "work",     archie: "" },
        { name: "notas.md",          icon: "doc",    cat: "documents", size: "3 KB",   kind: "",        when: "11 oct 22:47", domain: "personal", archie: "" },
        { name: "esquema.kicad_pcb", icon: "cad",    cat: "other",     size: "910 KB", kind: "",        when: "hoy 13:56",    domain: "",         archie: "13:56" },
        { name: "boceto.svg",        icon: "image",  cat: "media",     size: "44 KB",  kind: "",        when: "5 oct 16:02",  domain: "",         archie: "", loading: true }
    ]
    readonly property var queue: ({ title: "Copiando a Entregas", detail: "3 de 9 archivos", progress: 1 / 3,
                                    permission: "Permiso concedido · lectura de /archie/workspaces/motor-bracket",
                                    undoWhy: "Deshacer: aún no hay nada que deshacer", updated: "actualizado hace 12 s" })
    readonly property var authorization: ({ title: "Archie pide escribir en motor-bracket", path: "/archie/workspaces/motor-bracket" })
    readonly property var audit: [
        { t: "14:09:12", text: "Policy concedió lectura de /archie/workspaces/motor-bracket (1 operación)", kind: "done" },
        { t: "14:08:40", text: "Policy rechazó una escritura fuera del espacio de trabajo", kind: "bad" },
        { t: "14:07:55", text: "Stage: Archie hizo clic en KiCad (recibo: executed)", kind: "archie" }
    ]
    readonly property var quick: [
        { name: "Wi-Fi", icon: "wifi" }, { name: "Bluetooth", icon: "bt" },
        { name: "Silencio", icon: "moon" }, { name: "Privacidad", icon: "eye" }
    ]
    // Contents of the two sample folders, for double-click navigation (still design data).
    readonly property var children: ({
        "motor-bracket": [
            { name: "soporte-v3.step",   icon: "cad",   cat: "other", size: "2,4 MB", kind: "", when: "hoy 14:18", domain: "", archie: "14:18" },
            { name: "esquema.kicad_pcb", icon: "cad",   cat: "other", size: "910 KB", kind: "", when: "hoy 13:56", domain: "", archie: "13:56" },
            { name: "render-final.mp4",  icon: "video", cat: "media", size: "86 MB",  kind: "", when: "10 oct 09:12", domain: "", archie: "", loading: true },
            { name: "notas.md",          icon: "doc",   cat: "documents", size: "3 KB", kind: "", when: "11 oct 22:47", domain: "personal", archie: "" } ],
        "fotos-viaje": [
            { name: "IMG_0412.jpg", icon: "image", cat: "media", size: "3,1 MB", kind: "", when: "3 oct 21:05", domain: "personal", archie: "" },
            { name: "IMG_0413.jpg", icon: "image", cat: "media", size: "2,8 MB", kind: "", when: "3 oct 21:06", domain: "personal", archie: "" },
            { name: "IMG_0420.jpg", icon: "image", cat: "media", size: "3,4 MB", kind: "", when: "3 oct 21:14", domain: "personal", archie: "" } ]
    })
    // What each sidebar place lists (sample data, no filesystem).
    function placeFiles(i) {
        var f = files
        if (i === 0) return f
        if (i === 1) return f.filter(function (x) { return x.icon === "folder" })
        if (i === 2) return f.filter(function (x) { return x.domain === "personal" })
        if (i === 3) return f.filter(function (x) { return x.domain === "work" })
        if (i === 4) return f.filter(function (x) { return x.archie !== "" || x.when.indexOf("hoy") === 0 || x.when.indexOf("ayer") === 0 })
        return []
    }
    // Catalogue of the palette: apps, the sample files and actions. `act` is what Main.run() does;
    // `why` marks an entry that has no contract yet (disabled). `pref` = shown with an empty query.
    readonly property var catalog: [
        { group: "Apps", icon: "dkOrb",    name: "Orb",      hint: "Escritorio con el Orbe", kind: "App", act: "app.orb" },
        { group: "Apps", icon: "dkVault",  name: "Vault",    hint: "Gestor de archivos", kind: "App", act: "app.vault" },
        { group: "Apps", icon: "dkForge",  name: "Forge",    hint: "Catálogo de puentes y pipeline", kind: "App", act: "app.forge" },
        { group: "Apps", icon: "dkSentry", name: "Sentry",   hint: "Red local · inventario de dispositivos", kind: "App", act: "app.sentry", pref: true },
        { group: "Apps", icon: "dkStage",  name: "Stage",    hint: "Ventana de composición", kind: "App", act: "app.stage", pref: true },
        { group: "Apps", icon: "dkSet",    name: "Ajustes",  hint: "Proveedores, modelos y diagnóstico", kind: "App", act: "app.set" },
        // Vault's places (VaultWindow.places, same order): read-only navigation.
        { group: "Carpetas", icon: "personal", name: "Inicio", hint: "Vault · tu carpeta personal", kind: "Carpeta", act: "go:0", pref: true },
        { group: "Carpetas", icon: "spaces", name: "Escritorio", hint: "Vault · Escritorio", kind: "Carpeta", act: "go:1" },
        { group: "Carpetas", icon: "documents", name: "Documentos", hint: "Vault · Documentos", kind: "Carpeta", act: "go:2" },
        { group: "Carpetas", icon: "deliveries", name: "Descargas", hint: "Vault · Descargas", kind: "Carpeta", act: "go:3" },
        { group: "Carpetas", icon: "image", name: "Imágenes", hint: "Vault · Imágenes", kind: "Carpeta", act: "go:4" },
        { group: "Carpetas", icon: "stack", name: "Sistema", hint: "Vault · la raíz del sistema", kind: "Carpeta", act: "go:5" },
        { group: "Acciones", icon: "activity", name: "Mostrar actividad de Archie", hint: "Abre el panel de trabajos y autorizaciones", kind: "Acción", act: "overlay.activity", pref: true },
        { group: "Acciones", icon: "windows",  name: "Abrir la vista general de ventanas", hint: "Muestra las ventanas por app y por dominio", kind: "Acción", act: "overlay.switcher" },
        { group: "Acciones", icon: "bell",     name: "Mostrar notificaciones", hint: "Historial de avisos de Archie", kind: "Acción", act: "overlay.notif" },
        { group: "Acciones", icon: "control",  name: "Abrir el centro de control", hint: "Estado del sistema y medidores", kind: "Acción", act: "overlay.control" },
        { group: "Acciones", icon: "trace",    name: "Abrir el inspector", hint: "IRIS y Cortex", kind: "Acción", act: "overlay.inspector" },
        { group: "Acciones", icon: "dkOrb",    name: "Cambiar al Orbe", hint: "Volver al escritorio con el Orbe", kind: "Acción", act: "app.orb", pref: true },
        { group: "Acciones", icon: "windows",  name: "Mostrar todas las ventanas", hint: "Restaura las ventanas minimizadas", kind: "Acción", act: "win.showAll" },
        { group: "Acciones", icon: "dkSet",    name: "Abrir los ajustes", hint: "Proveedores, modelos, autoridad y apariencia", kind: "Acción", act: "app.set" }
    ]
    function search(q) {
        q = (q || "").trim().toLowerCase()
        return catalog.filter(function (c) {
            return q === "" ? c.pref === true : (c.name + " " + c.hint).toLowerCase().indexOf(q) >= 0 })
    }
    readonly property string query: "s"
    readonly property var results: [
        { group: "Apps",     icon: "dkSentry", name: "Sentry",   hint: "Red local · inventario de dispositivos", kind: "App" },
        { group: "Apps",     icon: "dkStage",  name: "Stage",    hint: "Ventana de composición",                kind: "App" },
        { group: "Archivos", icon: "file",     name: "soporte-v3.step", hint: "Vault · Diseño / Mecánica · modificado hoy 13:56", kind: "Archivo" },
        { group: "Acciones", icon: "activity", name: "Mostrar actividad de Archie", hint: "Abre el panel de trabajos y autorizaciones", kind: "Acción" },
        { group: "Acciones", icon: "dkOrb",    name: "Cambiar al Orbe", hint: "Volver al modo principal", kind: "Acción", why: "sin contrato todavía" }
    ]
}
