//! Views linked to scripts: a script's `state.name = value` reaches every
//! view declaring `state name`, including views opened later, and a view's
//! changes reach scripts.

use aegle_loader::{
    Data, View,
    markup::{Record, Type},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

struct Linked {
    view: Weak<RefCell<View>>,
    /// The entry states, from the program the view was last built from.
    states: Vec<(String, Type)>,
    /// States already reporting changes; handlers follow reloads that keep
    /// a state's name and type.
    hooked: Vec<(String, Type)>,
}

type Fields = Rc<[(String, Type)]>;

thread_local! {
    static LINKED: RefCell<Vec<Linked>> = const { RefCell::new(Vec::new()) };
    /// Record fields by record name, from every loaded program.
    static RECORDS: RefCell<HashMap<String, Fields>> = RefCell::default();
}

/// Remembers the record declarations of a loaded program.
pub(crate) fn learn(records: &[Record]) {
    RECORDS.with_borrow_mut(|known| {
        for record in records {
            known.insert(record.name.clone(), record.fields.clone().into());
        }
    });
}

/// The fields of the record named `name`.
pub(crate) fn fields(name: &str) -> Option<Fields> {
    RECORDS.with_borrow(|known| known.get(name).cloned())
}

/// Links a freshly built or reloaded view with its entry `states`: it takes
/// the values scripts have set and reports its changes to them.
pub(crate) fn link(shared: &Rc<RefCell<View>>, states: &[(String, Type)]) {
    let fresh = LINKED.with_borrow_mut(|linked| {
        linked.retain(|l| l.view.strong_count() > 0);
        let index = match linked
            .iter()
            .position(|l| std::ptr::eq(l.view.as_ptr(), Rc::as_ptr(shared)))
        {
            Some(index) => index,
            None => {
                linked.push(Linked {
                    view: Rc::downgrade(shared),
                    states: Vec::new(),
                    hooked: Vec::new(),
                });
                linked.len() - 1
            }
        };
        let entry = &mut linked[index];
        entry.states = states.to_vec();
        let fresh: Vec<_> = states
            .iter()
            .filter(|s| !entry.hooked.contains(s))
            .cloned()
            .collect();
        entry.hooked.extend(fresh.iter().cloned());
        fresh
    });
    let view = shared.borrow();
    for (name, ty) in states {
        match crate::script::shared(name, ty) {
            Some(value) => {
                if let Err(e) = view.set(name, value) {
                    eprintln!("palettes: state `{name}`: {e}");
                }
            }
            None => {
                if let Some(value) = view.get(name) {
                    crate::script::initial(name, ty, &value);
                }
            }
        }
    }
    for (name, ty) in fresh {
        let Some(state) = view.state::<Data>(&name) else {
            continue;
        };
        state.on_change(move |value: Data| crate::script::changed(&name, &ty, &value));
    }
}

/// Sets `name` in every linked view declaring it, converting the value to
/// each view's type with `convert`.
pub(crate) fn assign(
    name: &str,
    mut convert: impl FnMut(&Type) -> Result<Data, String>,
) -> Result<(), String> {
    let targets: Vec<(Rc<RefCell<View>>, Type)> = LINKED.with_borrow(|linked| {
        linked
            .iter()
            .filter_map(|l| {
                let (_, ty) = l.states.iter().find(|(n, _)| n == name)?;
                Some((l.view.upgrade()?, ty.clone()))
            })
            .collect()
    });
    for (view, ty) in targets {
        let value = convert(&ty)?;
        // A view being rebuilt takes the value when it links again.
        if let Ok(view) = view.try_borrow() {
            view.set(name, value).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
