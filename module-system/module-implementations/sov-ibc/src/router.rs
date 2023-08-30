use std::cell::RefCell;
use std::rc::Rc;

use ibc::applications::transfer::{MODULE_ID_STR, PORT_ID_STR};
use ibc::core::ics24_host::identifier::PortId;
use ibc::core::router::{self, ModuleId, Router};
use sov_ibc_transfer::context::TransferContext;
use sov_state::WorkingSet;

use crate::IbcModule;

pub struct IbcRouter<'ws, 'c, C: sov_modules_api::Context> {
    pub transfer_ctx: TransferContext<'ws, 'c, C>,
}

impl<'t, 'ws, 'c, C> IbcRouter<'ws, 'c, C>
where
    C: sov_modules_api::Context,
{
    pub fn new(
        ibc_mod: &'t IbcModule<C>,
        sdk_context: &'c C,
        working_set: Rc<RefCell<&'ws mut WorkingSet<C::Storage>>>,
    ) -> IbcRouter<'ws, 'c, C> {
        IbcRouter {
            transfer_ctx: ibc_mod
                .transfer
                .clone()
                .into_context(sdk_context, working_set),
        }
    }
}

impl<'ws, 'c, C> Router for IbcRouter<'ws, 'c, C>
where
    C: sov_modules_api::Context,
{
    fn get_route(&self, module_id: &ModuleId) -> Option<&dyn router::Module> {
        if *module_id == ModuleId::new(MODULE_ID_STR.to_string()) {
            Some(&self.transfer_ctx)
        } else {
            None
        }
    }

    fn get_route_mut(&mut self, module_id: &ModuleId) -> Option<&mut dyn router::Module> {
        if *module_id == ModuleId::new(MODULE_ID_STR.to_string()) {
            Some(&mut self.transfer_ctx)
        } else {
            None
        }
    }

    fn lookup_module(&self, port_id: &PortId) -> Option<ModuleId> {
        if port_id.as_str() == PORT_ID_STR {
            Some(ModuleId::new(MODULE_ID_STR.to_string()))
        } else {
            None
        }
    }
}
