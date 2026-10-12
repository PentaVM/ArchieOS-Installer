pragma Singleton
import QtQuick

// SYNTHETIC DESIGN DATA. Nothing here comes from archied, Sentry or the filesystem.
// Every screen that uses it shows the DESIGN DATA banner. It is replaced by the real
// adapter in phase C (see docs/gui/INTEGRATION_MATRIX.md).
QtObject {
    readonly property var dock: [
        { id: "orb",    name: "Orb",     glyph: "◉", color: "#6fe3f5", running: true  },
        { id: "vault",  name: "Vault",   glyph: "▤", color: "#5aa9ff", running: true  },
        { id: "forge",  name: "Forge",   glyph: "✱", color: "#f0b429", running: false },
        { id: "sentry", name: "Sentry",  glyph: "◎", color: "#47d18b", running: false },
        { id: "stage",  name: "Stage",   glyph: "▣", color: "#c08cff", running: true  },
        { id: "set",    name: "Ajustes", glyph: "≡", color: "#aab3c0", running: false }
    ]
    readonly property var places: [
        { name: "Entregas",   icon: "⇩", note: "~/Archie",            domain: "" },
        { name: "Workspaces", icon: "▦", note: "/archie/workspaces",  domain: "" },
        { name: "Documentos", icon: "▤", note: "",                    domain: "personal" },
        { name: "Trabajo",    icon: "▤", note: "",                    domain: "work" },
        { name: "Recientes",  icon: "◷", note: "",                    domain: "" },
        { name: "Papelera",   icon: "✕", note: "",                    domain: "" }
    ]
    readonly property var files: [
        { name: "motor-bracket",     kind: "folder", size: "",       when: "hoy 09:14", domain: "",         archie: true  },
        { name: "soporte-v3.step",   kind: "cad",    size: "2,4 MB", when: "hoy 09:12", domain: "",         archie: true  },
        { name: "informe-q3.docx",   kind: "doc",    size: "148 KB", when: "ayer",      domain: "work",     archie: false },
        { name: "render-final.mp4",  kind: "video",  size: "86 MB",  when: "ayer",      domain: "",         archie: true  },
        { name: "fotos-viaje",       kind: "folder", size: "",       when: "lun",       domain: "personal", archie: false },
        { name: "presupuesto.xlsx",  kind: "sheet",  size: "62 KB",  when: "lun",       domain: "work",     archie: false },
        { name: "notas.md",          kind: "text",   size: "3 KB",   when: "dom",       domain: "personal", archie: false },
        { name: "esquema.kicad_pcb", kind: "cad",    size: "910 KB", when: "dom",       domain: "",         archie: true  },
        { name: "boceto.svg",        kind: "image",  size: "44 KB",  when: "sáb",       domain: "",         archie: false }
    ]
    readonly property var jobs: [
        { name: "Barrido FreeCAD · soporte",  state: "running", progress: 0.62, detail: "iteración 31 de 50" },
        { name: "Copiar a Entregas",          state: "running", progress: 0.35, detail: "3 de 9 archivos" },
        { name: "Render timeline",            state: "done",    progress: 1.0,  detail: "render-final.mp4" },
        { name: "Instalar herramienta kicad", state: "needs",   progress: 0.0,  detail: "espera tu aprobación" }
    ]
    readonly property var audit: [
        { t: "09:14", text: "Policy concedió lectura de /archie/workspaces/motor-bracket (1 operación)", kind: "ok"   },
        { t: "09:12", text: "Policy rechazó una escritura fuera del espacio de trabajo",              kind: "bad"  },
        { t: "09:10", text: "Stage: Archie hizo clic en KiCad (recibo: executed)",                    kind: "info" }
    ]
    readonly property var quick: [
        { name: "Wi-Fi",      state: "Conectado",               on: true  },
        { name: "Bluetooth",  state: "Apagado",                 on: false },
        { name: "Silencio",   state: "Desactivado",             on: false },
        { name: "Privacidad", state: "Cámara y micro inactivos", on: true }
    ]
    readonly property var results: [
        { kind: "App",     name: "Vault",                       hint: "Abrir gestor de archivos" },
        { kind: "App",     name: "KiCad",                       hint: "Archie puede operarla" },
        { kind: "Archivo", name: "soporte-v3.step",             hint: "Entregas / motor-bracket" },
        { kind: "Acción",  name: "Mostrar actividad de Archie", hint: "Centro de actividad" },
        { kind: "Acción",  name: "Cambiar al Orbe",             hint: "Volver al modo principal" }
    ]
}
