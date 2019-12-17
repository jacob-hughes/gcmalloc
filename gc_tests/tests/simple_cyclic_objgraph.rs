// Run-time:
//   status: success

extern crate gcmalloc;

use gcmalloc::{collect, DebugFlags, Debug, Gc};

struct Node {
    data: String,
    edge: Option<Gc<Node>>,
}

fn make_objgraph() -> Gc<Node> {
    let mut a = Gc::new(Node {
        data: "a".to_string(),
        edge: None,
    });
    let mut b = Gc::new(Node {
        data: "b".to_string(),
        edge: None,
    });
    let mut c = Gc::new(Node {
        data: "c".to_string(),
        edge: None,
    });

    a.edge = Some(b);
    b.edge = Some(c);
    c.edge = Some(a);

    a
}

fn main() {
    gcmalloc::debug_flags(DebugFlags::new().sweep_phase(false));

    let a = make_objgraph();
    gcmalloc::collect();

    // Test a
    assert_eq!(a.data, String::from("a"));
    assert!(Debug::is_black(a.as_ptr() as *mut u8));

    // Test b
    assert_eq!(a.edge.unwrap().data, String::from("b"));
    assert!(Debug::is_black(a.edge.unwrap().as_ptr() as *mut u8));

    // Test c
    assert_eq!(a.edge.unwrap().edge.unwrap().data, String::from("c"));
    assert!(Debug::is_black(
        a.edge.unwrap().edge.unwrap().as_ptr() as *mut u8
    ));

    // Test c -> a
    assert_eq!(
        a.edge.unwrap().edge.unwrap().edge.unwrap().data,
        String::from("a")
    );
    assert!(Debug::is_black(
        a.edge.unwrap().edge.unwrap().edge.unwrap().as_ptr() as *mut u8
    ));
}
