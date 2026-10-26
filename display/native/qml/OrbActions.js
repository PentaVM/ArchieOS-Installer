.pragma library

// Presentation only: callers own the live view; a missing node is not an applied action.
function perform(action,args,controller,scene,hud,neural) {
    if (action === "focus" || action === "highlight") {
        if (neural) return (action === "focus" ? neural.focusNode(args.id) : neural.highlightNode(args.id,args.enabled)) ? "" : "El nodo no está en esta instantánea"
        if (args.id !== "" && !controller.containsNode(args.id)) return "El nodo no está en el grafo visible"
        if (action === "focus") { if(args.id === "") controller.clearSelection(); else controller.select(args.id) }
        else controller.hover(args.enabled ? args.id : "")
        return ""
    }
    if (action === "pointer") {
        if (!args.click) return ""
        if (neural) return neural.clickAt(args.x*neural.width,args.y*neural.height) ? "" : "No hay un nodo en ese punto"
        var id=hud.pickAt(args.x*hud.width,args.y*hud.height)
        if (!id || !controller.containsNode(id)) return "No hay un nodo en ese punto"
        controller.select(id); return ""
    }
    if (action === "camera") {
        if (neural) return neural.moveCamera(args.move) ? "" : "Movimiento de cámara no disponible"
        if(args.move === "home") controller.resetProfile()
        else if(args.move === "closer") scene.approach(0.8)
        else if(args.move === "farther") scene.approach(1/0.8)
        else if(args.move === "left") scene.yaw-=15
        else if(args.move === "right") scene.yaw+=15
        else if(args.move === "up") scene.pitch=Math.min(80,scene.pitch+10)
        else if(args.move === "down") scene.pitch=Math.max(-80,scene.pitch-10)
        else return "Movimiento de cámara no disponible"
        return ""
    }
    return "La vista no admite esta acción"
}
