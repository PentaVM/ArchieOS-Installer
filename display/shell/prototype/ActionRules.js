.pragma library

// Local presentation actions are distinct from authority-bearing operations.
function allowed(id, context) {
    context = context || {}
    if (id === "orb.nav") return context.nativeReady === true
    if (id === "app.stage") return context.stageLauncher === true
    if (id === "app.world") return context.worldHost === true
    if (id.indexOf("dock.") === 0 || id.indexOf("window.") === 0)
        return ["vault","forge","sentry","stage","set"].indexOf(id.slice(id.indexOf(".") + 1)) >= 0
    // Vault reads the person's folders; it never creates, moves, copies or deletes a file.
    if (/^go:[0-5]$/.test(id)) return true
    if (["app.orb","app.vault","app.forge","app.sentry","app.set", "desk.toggle",
         "vault.newTab","vault.closeTab","vault.sort","vault.list","vault.grid","vault.open","vault.up",
         "theme.motion","theme.opaque", "win.close","win.min","win.zoom",
         "win.closeAll","win.minAll","win.showAll","win.desktop"].indexOf(id) >= 0) return true
    if (id === "theme.blur") return context.compositorBlur === true
    if (id.indexOf("overlay.") === 0)
        return ["activity","control","forge","inspector","settings","sentry","search",
                "palette","switcher","notif","workspace"].indexOf(id.slice(8)) >= 0
    return false
}
function reason(id, context) {
    if (allowed(id, context)) return ""
    if (id === "overlay.lock") return "No hay un bloqueo de sesión conectado; una pantalla de muestra no bloquea el equipo."
    if (id.indexOf("vault.") === 0 || id.indexOf("file:") === 0 || id.indexOf("go:") === 0)
        return "Vault solo lee archivos: no copia, mueve ni borra."
    if (id === "orb.nav") return "El viewport nativo aún no está disponible."
    if (id === "app.stage") return "El lanzador de Stage no está disponible en este host."
    if (id === "app.world") return "World no está disponible en este host."
    if (id === "theme.blur") return "El compositor no ha declarado soporte de desenfoque."
    return "Acción sin contrato disponible."
}
