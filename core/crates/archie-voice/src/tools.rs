//! What Archie can be asked to do on the computer, described for the model.
//!
//! Each of these is offered to the cognitive model so it can *propose* it. None of them
//! does anything here: a proposal becomes a [`archie_speech::SpeechToolIntentCandidate`],
//! and whether it happens is decided by Cerebra and authorised by Policy, exactly as with
//! any other intent. The descriptions are written for the model in the language the
//! person speaks, because a model picks tools by reading their descriptions.
//!
//! The set is the stage's own vocabulary and nothing more — open, click, type, press,
//! scroll — so that every proposal maps to an action the stage already checks, leases and
//! can refuse.

use archie_model_runtime::ToolDefinition;
use serde_json::json;

/// The tools that operate the computer.
pub fn os_control() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "open_app".into(),
            description: "Abre una aplicación del sistema por su nombre, por ejemplo \
                          firefox, terminal o el editor de texto."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {"app": {"type": "string", "description": "Nombre de la aplicación."}},
                "required": ["app"]
            }),
        },
        ToolDefinition {
            name: "click".into(),
            description: "Hace clic en algo que se ve en la pantalla, nombrado como aparece: \
                          un botón por su texto (por ejemplo Guardar), un campo por su \
                          etiqueta, un enlace, un menú. Úsalo para seleccionar un campo antes \
                          de escribir en él y para pulsar botones."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "target": {"type": "string", "description": "Lo que hay que tocar, como se ve."},
                    "app": {"type": "string", "description": "En qué aplicación está, si se sabe."}
                },
                "required": ["target"]
            }),
        },
        ToolDefinition {
            name: "type_text".into(),
            description: "Escribe texto con el teclado en el campo que tenga el foco. Si el \
                          campo no está seleccionado, primero haz clic en él con click."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        },
        ToolDefinition {
            name: "press_keys".into(),
            description: "Pulsa una tecla o una combinación, por ejemplo enter, escape o ctrl+s."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {"keys": {"type": "string"}},
                "required": ["keys"]
            }),
        },
        ToolDefinition {
            name: "scroll".into(),
            description: "Desplaza la vista hacia arriba o hacia abajo.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "direction": {"type": "string", "enum": ["up", "down"]},
                    "amount": {"type": "integer", "minimum": 1, "maximum": 20}
                },
                "required": ["direction"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_a_named_object_with_its_requirements_stated() {
        let tools = os_control();
        let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), tools.len(), "no two tools share a name");
        for tool in &tools {
            assert_eq!(tool.parameters["type"], "object", "{}", tool.name);
            assert!(
                tool.parameters["required"]
                    .as_array()
                    .is_some_and(|r| !r.is_empty()),
                "{} says what it needs",
                tool.name
            );
            assert!(
                tool.description.len() > 20,
                "{} says what it does",
                tool.name
            );
        }
    }
}
