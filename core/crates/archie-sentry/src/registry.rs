use crate::contracts::{validate_id, CommandKind, ContractError, Intent};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceDescriptor {
    pub resource_id: String,
    pub edge_id: String,
    pub adapter_id: String,
    pub paired: bool,
    pub capabilities: Vec<CommandKind>,
}

#[derive(Debug, Default)]
pub struct Registry {
    resources: BTreeMap<String, ResourceDescriptor>,
}
impl Registry {
    pub fn register(&mut self, resource: ResourceDescriptor) -> Result<(), ContractError> {
        validate_id(&resource.resource_id)?;
        validate_id(&resource.edge_id)?;
        validate_id(&resource.adapter_id)?;
        if resource.capabilities.is_empty()
            || resource.capabilities.len() > 5
            || resource.capabilities.iter().collect::<BTreeSet<_>>().len()
                != resource.capabilities.len()
        {
            return Err(ContractError("invalid capability declaration"));
        }
        if self.resources.contains_key(&resource.resource_id) {
            return Err(ContractError("resource already registered"));
        }
        if self.resources.len() >= 4096 {
            return Err(ContractError("resource registry full"));
        }
        self.resources
            .insert(resource.resource_id.clone(), resource);
        Ok(())
    }
    pub fn resolve(
        &self,
        intent: &Intent,
        edge: &str,
        adapter: &str,
    ) -> Result<&ResourceDescriptor, ContractError> {
        intent.validate()?;
        let resource = self
            .resources
            .get(&intent.resource_id)
            .ok_or(ContractError("unknown resource"))?;
        if !resource.paired {
            return Err(ContractError("resource is not paired"));
        }
        if resource.edge_id != edge || resource.adapter_id != adapter {
            return Err(ContractError("adapter route mismatch"));
        }
        if !resource.capabilities.contains(&intent.command.capability()) {
            return Err(ContractError(
                "resource does not declare requested capability",
            ));
        }
        Ok(resource)
    }
    pub fn set_paired(&mut self, id: &str, paired: bool) -> Result<(), ContractError> {
        self.resources
            .get_mut(id)
            .ok_or(ContractError("unknown resource"))?
            .paired = paired;
        Ok(())
    }
}
