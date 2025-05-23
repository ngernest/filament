use super::Fsm;
use super::fsm::{FsmBind, FsmType};
use super::utils::{NameGenerator, cell_to_port_def};
use calyx_ir::{self as calyx, RRC};
use fil_ir::{self as ir, Ctx, DenseIndexInfo, DisplayCtx};
use fil_utils::{self as utils, AttrCtx};
use itertools::Itertools;
use std::{collections::HashMap, rc::Rc};

#[derive(Default)]
/// Bindings associated with the current compilation context
pub(super) struct Binding {
    // Component signatures
    comps: HashMap<ir::CompIdx, RRC<calyx::Cell>>,
    /// Mapping to the component representing FSM with particular number of states
    pub fsm_comps: FsmBind,
}

impl Binding {
    /// Inserts a [calyx::Cell] into the binding
    pub fn insert(&mut self, name: ir::CompIdx, sig: RRC<calyx::Cell>) {
        self.comps.insert(name, sig);
    }

    /// Gets a [calyx::Cell]'s signature from an [ir::CompIdx]
    pub fn get(&self, idx: &ir::CompIdx) -> Option<Vec<calyx::PortDef<u64>>> {
        self.comps.get(idx).map(cell_to_port_def)
    }
}

/// Contains the context needed to compile a component.
pub(super) struct BuildCtx<'a> {
    pub builder: calyx::Builder<'a>,
    pub binding: &'a mut Binding,
    pub comp: &'a ir::Component,
    ctx: &'a ir::Context,
    lib: &'a calyx::LibrarySignatures,
    /// Helper to generate names
    ng: &'a NameGenerator,
    /// Mapping from events to the FSM that reify them.
    fsms: HashMap<ir::EventIdx, Fsm>,
    /// Mapping from [ir::InstIdx]s to the calyx cell instantiated.
    instances: DenseIndexInfo<ir::Instance, RRC<calyx::Cell>>,
    /// Mapping from [ir::InstIdx]s to a reference of the calyx cell instantiated/invoked
    invokes: DenseIndexInfo<ir::Invoke, RRC<calyx::Cell>>,
}

impl<'a> BuildCtx<'a> {
    pub fn new(
        ctx: &'a ir::Context,
        idx: ir::CompIdx,
        binding: &'a mut Binding,
        ng: &'a NameGenerator,
        builder: calyx::Builder<'a>,
        lib: &'a calyx::LibrarySignatures,
    ) -> Self {
        BuildCtx {
            ctx,
            ng,
            comp: ctx.get(idx),
            binding,
            builder,
            lib,
            instances: DenseIndexInfo::default(),
            invokes: DenseIndexInfo::default(),
            fsms: HashMap::new(),
        }
    }

    /// Adds an instance to the component
    pub fn add_instance(&mut self, idx: ir::InstIdx) {
        let inst = self.comp.get(idx);
        // generate a unique name for this instance
        let inst_name = self.ng.instance_name(idx, self.comp);
        let comp_name = self.ng.comp_name(inst.comp, self.ctx);

        let cell = if let Some(sig) = self.binding.get(&inst.comp) {
            // this component has is in the binding signature (it has been compiled and is non-primitive)
            self.builder.add_component(
                inst_name, // non-primitive component
                comp_name, sig,
            )
        } else {
            // this instance must be referring to a primitive, so we add one to the component

            // gets the parameters of this instance as concrete numbers
            let conc_bind = inst
                .args
                .iter()
                .map(|v| v.concrete(self.comp))
                .collect_vec();
            self.builder.add_primitive(inst_name, comp_name, &conc_bind)
        };

        cell.borrow_mut()
            .attributes
            .insert(calyx::BoolAttr::Data, 1);

        // add this instance to the instance mapping
        self.instances.push(idx, cell);
    }

    /// Adds an invocation to the component
    pub fn add_invoke(&mut self, invidx: ir::InvIdx) {
        let inv = self.comp.get(invidx);

        // Gets a reference to the instance being invoked
        let cell = &self.instances[inv.inst];

        // loop through the event bindings defined in the instance and connect them to the corresponding fsms.
        for eb in inv.events.iter() {
            // If there is no interface port, no binding necessary
            if let Some(dst) = eb.base.apply(
                |ev: ir::EventIdx, comp: &ir::Component| {
                    self.ng.interface_name(ev, comp)
                },
                self.ctx,
            ) {
                let ir::EventBind { arg: time, .. } = eb;

                // gets the interface port from the signature of the instance
                let dst = cell.borrow().get(dst);

                let time = self.comp.get(*time);
                let offset = time.offset.concrete(self.comp);
                // finds the corresponding port on the fsm of the referenced event
                let src = self.fsms.get(&time.event).unwrap().range_guard(
                    &mut self.builder,
                    offset,
                    offset + 1,
                );

                let c = self.builder.add_constant(1, 1);

                // builds the assignment `dst = src ? 1'd1;`
                let assign = self.builder.build_assignment(
                    dst,
                    c.borrow().get("out"),
                    src,
                );
                self.builder.component.continuous_assignments.push(assign);
            }
        }

        // add a copy of the instance pointer to the invoke mapping
        self.invokes.push(invidx, Rc::clone(cell));
    }

    /// Converts an interval to a guard expression with the appropriate FSM
    /// Returns no guard if the related event has no interface port.
    pub fn compile_range(
        &mut self,
        range: &ir::Range,
    ) -> calyx::Guard<calyx::Nothing> {
        let start = self.comp.get(range.start);
        let end = self.comp.get(range.end);

        assert!(
            start.event == end.event,
            "Range `{}` cannot be represented as a simple offset",
            self.comp.display(range)
        );

        let ev = start.event;

        // Don't generate a guard if there is no interface port
        if !self.comp.get(ev).has_interface {
            return calyx::Guard::True;
        }

        // return a guard that is active whenever from for all states from `start..end`
        let fsm = self.fsms.get(&ev).unwrap();
        fsm.range_guard(
            &mut self.builder,
            start.offset.concrete(self.comp),
            end.offset.concrete(self.comp),
        )
    }

    /// Compiles an [ir::Port], returning the proper guard if present.
    pub fn compile_port(
        &mut self,
        idx: ir::PortIdx,
    ) -> (RRC<calyx::Port>, calyx::Guard<calyx::Nothing>) {
        let port = self.comp.get(idx);

        let name = self.ng.port_name(idx, self.ctx, self.comp);

        let guard = self.compile_range(&port.live.range);
        let cell = match port.owner {
            ir::PortOwner::Sig { .. } => {
                self.builder.component.signature.borrow()
            }
            ir::PortOwner::Inv { inv, .. } => self.invokes[inv].borrow(),
            ir::PortOwner::Local => {
                unreachable!("Local ports should have been eliminated.")
            }
        };
        (cell.get(name), guard)
    }

    /// Compiles an [ir::Connect] by building the port assignments in calyx
    pub fn compile_connect(&mut self, con: &ir::Connect) {
        let ir::Connect { dst, src, .. } = con;

        assert!(
            src.is_port(self.comp),
            "Bundles should have been compiled away."
        );

        assert!(
            dst.is_port(self.comp),
            "Bundles should have been compiled away."
        );

        log::debug!("Compiling connect: {}", self.comp.display(con));

        // ignores the guard of the src (bind check already verifies that it is available for at least as long as dest)
        let (dst, g) = self.compile_port(dst.port);
        let (src, _) = self.compile_port(src.port);
        let assign = self.builder.build_assignment(dst, src, g);
        self.builder.component.continuous_assignments.push(assign);
    }

    /// Attempts to declare an fsm component (if not already declared) in the [Binding] stored by this [BuildCtx]
    /// and creates an [Fsm] from this [calyx::Component] FSM and stores it in the [BuildCtx]
    pub fn insert_fsm(&mut self, event: ir::EventIdx, states: u64) {
        let evt = self.comp.get(event);
        // Construct an fsm iff the event is connected to an interface port
        if evt.has_interface {
            let ir::TimeSub::Unit(delay) = evt.delay else {
                self.comp.internal_error(
                    "Non-unit delays should have been compiled away.",
                );
            };
            let delay = delay.concrete(self.comp);
            let typ =
                match self.comp.attrs.get(utils::CompBool::CounterFSM).unwrap()
                {
                    false => FsmType::Simple(states),
                    true => FsmType::CounterChain(states, delay),
                };
            self.implement_fsm(&typ);

            // Construct the FSM
            let fsm = Fsm::new(event, typ, self, self.ng);
            self.fsms.insert(event, fsm);
        }
    }

    /// Creates a [calyx::Component] representing an FSM that supports a given number of states and delay.
    /// Adds component to the [Binding] held by this component.
    fn implement_fsm(&mut self, typ: &super::FsmType) {
        self.binding.fsm_comps.add(typ, self.lib);
    }
}
