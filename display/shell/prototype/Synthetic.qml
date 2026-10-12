pragma Singleton
import QtQuick

// SYNTHETIC DESIGN DATA, taken from the Archie Mac mockups. Nothing here comes from archied,
// Sentry or the filesystem; every screen that uses it says "Diseño" (the DATOS DE DISEÑO notice
// and per-section chips). The bridge-fed parts (link, control center, activity jobs, Sentry)
// do not read from here.
QtObject {
    readonly property string clock: "lun 12 oct  14:32"
    readonly property var dock: [
        { id: "orb",    name: "Orb",     icon: "dkOrb",    running: true  },
        { id: "vault",  name: "Vault",   icon: "dkVault",  running: true  },
        { id: "forge",  name: "Forge",   icon: "dkForge",  running: false },
        { id: "sentry", name: "Sentry",  icon: "dkSentry", running: false },
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
    readonly property string query: "s"
    readonly property var results: [
        { group: "Apps",     icon: "dkSentry", name: "Sentry",   hint: "Red local · inventario de dispositivos", kind: "App" },
        { group: "Apps",     icon: "dkStage",  name: "Stage",    hint: "Ventana de composición",                kind: "App" },
        { group: "Archivos", icon: "file",     name: "soporte-v3.step", hint: "Vault · Diseño / Mecánica · modificado hoy 13:56", kind: "Archivo" },
        { group: "Acciones", icon: "activity", name: "Mostrar actividad de Archie", hint: "Abre el panel de trabajos y autorizaciones", kind: "Acción" },
        { group: "Acciones", icon: "dkOrb",    name: "Cambiar al Orbe", hint: "Volver al modo principal", kind: "Acción", why: "sin contrato todavía" }
    ]
}
