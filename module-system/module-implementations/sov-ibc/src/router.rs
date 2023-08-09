use ibc::core::router::Router;

#[derive(Default)]
pub struct IbcRouter;

impl Router for IbcRouter {
    fn get_route(
        &self,
        module_id: &ibc::core::router::ModuleId,
    ) -> Option<&dyn ibc::core::router::Module> {
        todo!()
    }

    fn get_route_mut(
        &mut self,
        module_id: &ibc::core::router::ModuleId,
    ) -> Option<&mut dyn ibc::core::router::Module> {
        todo!()
    }

    fn lookup_module(
        &self,
        port_id: &ibc::core::ics24_host::identifier::PortId,
    ) -> Option<ibc::core::router::ModuleId> {
        todo!()
    }
}
