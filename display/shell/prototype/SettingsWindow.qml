import QtQuick

// Settings, 940 px window. DESIGN SAMPLE. Four sections, all read-only: provider NAMES (only the
// name of the environment variable, never its value), models with their declared locality and
// privacy grade, the apps Archie may operate (policy, read-only: editing does not exist), and a
// short diagnosis. No control here writes anything.
AppWindow {
    id: sw
    title: "Ajustes"
    property string st: Demo.state
    property int section: 0
    readonly property var titles: ["Proveedores", "Modelos", "Apps que Archie puede operar", "Diagnóstico"]
    readonly property var nouns: ["elementos", "modelos", "apps", "comprobaciones"]
    readonly property bool plain: st === "normal" || st === "stale" || st === "degraded" || st === "offline"
    readonly property bool dim: st === "stale"
    readonly property bool unread: st === "offline"

    trailing: [ Chip { design: true; icon: "sample"; text: "Diseño · datos de ejemplo" } ]
    sidebar: [
        Eyebrow { text: "Ajustes"; leftPadding: 10; bottomPadding: 6 },
        SideRow { icon: "key"; text: "Proveedores"; on: sw.section === 0; ring: Demo.focusRing; onClicked: sw.section = 0 },
        SideRow { icon: "cpu"; text: "Modelos"; on: sw.section === 1; onClicked: sw.section = 1 },
        SideRow { icon: "lock"; text: "Apps de Archie"; on: sw.section === 2; onClicked: sw.section = 2 },
        SideRow { icon: "activity"; text: "Diagnóstico"; on: sw.section === 3; onClicked: sw.section = 3 } ]

    Item { width: parent.width; height: 24
        Eyebrow { text: sw.titles[sw.section]; anchors.verticalCenter: parent.verticalCenter }
        StateChip { st: sw.st; noun: sw.nouns[sw.section]; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter } }

    ErrBox { visible: sw.st === "error"; text: sw.section === 0 ? "No se pudo leer la lista de proveedores: respuesta no válida."
        : sw.section === 1 ? "No se pudo leer el registro de modelos: respuesta no válida."
        : sw.section === 2 ? "No se pudo leer la política de apps: respuesta no válida." : "No se pudo leer el diagnóstico: respuesta no válida." }
    VoidBox { visible: sw.st === "vacio"
        icon: ["key", "cpu", "lock", "activity"][sw.section]
        title: ["Ningún proveedor configurado", "Ningún modelo registrado", "Ninguna app en la política", "Sin comprobaciones"][sw.section]
        text: ["Cuando haya uno, aquí se verá solo el nombre de su variable de entorno.", "Cuando se registre uno, aquí se verá su localidad y su grado de privacidad.",
               "Cuando haya una política, aquí se verá en solo lectura.", "Cuando haya lecturas, aquí aparecerá cada servicio con su estado."][sw.section] }

    Column { visible: sw.plain && sw.section === 0; width: parent.width; opacity: sw.dim ? 0.62 : 1
        ListRow { glyph: "key"; title: "Proveedor A"; ring: Demo.focusRing; sub: "variable de entorno PROVEEDOR_A_API_KEY"
            Chip { small: true; tone: sw.unread ? "unk" : "ok"; icon: sw.unread ? "unknown" : "done"; text: sw.unread ? "sin lectura" : "definida" } }
        ListRow { glyph: "key"; title: "Proveedor B"; sub: sw.st === "degraded" ? "bridge sin datos" : "variable de entorno PROVEEDOR_B_API_KEY"
            Chip { small: true; tone: (sw.unread || sw.st === "degraded") ? "unk" : "neutral"; icon: (sw.unread || sw.st === "degraded") ? "unknown" : "stale"
                text: sw.unread ? "sin lectura" : (sw.st === "degraded" ? "no medido" : "sin definir") } } }
    Column { visible: sw.plain && sw.section === 1; width: parent.width; opacity: sw.dim ? 0.62 : 1
        ListRow { glyph: "cpu"; title: "Modelo local 1"; sub: "modelo registrado"
            Chip { small: true; tone: sw.unread ? "unk" : "ok"; icon: sw.unread ? "unknown" : "dkOrb"; text: sw.unread ? "sin lectura" : "local" }
            Chip { small: true; tone: sw.unread ? "unk" : "ok"; icon: sw.unread ? "unknown" : "lock"; text: sw.unread ? "sin lectura" : "privacidad alta" } }
        ListRow { glyph: "cpu"; title: "Modelo en la nube 1"; sub: "modelo registrado"
            Chip { small: true; tone: sw.unread ? "unk" : "neutral"; icon: sw.unread ? "unknown" : "cloud"; text: sw.unread ? "sin lectura" : "nube" }
            Chip { small: true; tone: sw.unread ? "unk" : "warn"; icon: sw.unread ? "unknown" : "warn"; text: sw.unread ? "sin lectura" : "privacidad media" } } }
    Column { visible: sw.plain && sw.section === 2; width: parent.width; opacity: sw.dim ? 0.62 : 1
        Repeater { model: [["dkVault", "Vault", "lectura"], ["dkForge", "Forge", "operar"], ["dkStage", "Stage", "operar"], ["dkSentry", "Sentry", "lectura"]]
            delegate: ListRow { glyph: modelData[0]; title: modelData[1]
                Chip { small: true; tone: sw.unread ? "unk" : "neutral"; icon: sw.unread ? "unknown" : "lock"; text: sw.unread ? "sin lectura" : modelData[2] } } } }
    Column { visible: sw.plain && sw.section === 3; width: parent.width; opacity: sw.dim ? 0.62 : 1
        ListRow { title: "archied"; Chip { small: true; tone: sw.unread ? "bad" : "ok"; icon: sw.unread ? "bad" : "ok"; text: sw.unread ? "no responde" : "en vivo" } }
        ListRow { title: "Policy"; Chip { small: true; tone: sw.unread ? "unk" : "ok"; icon: sw.unread ? "unknown" : "ok"; text: sw.unread ? "sin lectura" : "en vivo" } }
        ListRow { title: "Cortex"; Chip { small: true; tone: "unk"; icon: "unknown"; text: "no medido" } }
        ListRow { title: "Presupuesto"; Chip { small: true; tone: "unk"; icon: "unknown"; text: "sin lectura" } } }

    NoteBox { visible: sw.st === "stale"; icon: "recents"; text: "Última lectura hace 18 min." }
    NoteBox { visible: sw.section === 0 && sw.st !== "error"; text: "Los valores de las claves nunca se muestran ni se teclean aquí: solo el nombre de la variable." }
    NoteBox { visible: sw.section === 1 && sw.plain; text: "Localidad y grado de privacidad son los declarados por cada modelo registrado." }
    NoteBox { visible: sw.section === 2 && sw.plain; text: "Solo lectura: editar la política no existe todavía." }
}
