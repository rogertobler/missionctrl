use super::*;
use serde_json::json;

fn doc() -> Value {
    json!({"spaces":[
        {"id":"sa","name":"alpha","folder":"/tmp/a","color":"teal","tabs":[
            {"id":"ta1","name":"one","root":{"id":"ta1","type":"row","children":[
                {"id":"p1","type":"block","content":{"terminal":"t1"}},
                {"id":"s1","type":"stack","active":"p2","children":[
                    {"id":"p2","type":"block","content":{"terminal":"t2"}},
                    {"id":"p3","type":"block","content":{"mod":"files","props":{"x":1}}}]}],
                "floating":[{"id":"f1","content":{"terminal":"t9"},"edge":"right","peek":12}]},
             "zoomed":"p1"},
            {"id":"ta2","name":"two","root":{"id":"ta2","type":"row","children":[
                {"id":"p4","type":"block","content":{"terminal":"t4"}}]}}]},
        {"id":"sb","name":"","folder":"/tmp/b","tabs":[
            {"id":"tb1","name":"","root":{"id":"tb1","type":"column","children":[
                {"id":"p5","type":"block","content":{"terminal":"t5"}}]}}]}],
        "future":{"kept":true}})
}

#[test]
fn fresh_ids_look_like_the_apps() {
    let a = fresh_id("sp");
    assert_eq!(a.len(), 34);
    assert!(a.starts_with("sp") && a[2..].chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, fresh_id("sp"));
}

#[test]
fn readable_documents_are_writable() {
    for ok in [
        Value::Null,
        json!({}),
        json!({"spaces":[]}),
        doc(),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t"}]}]}),
        json!({"spaces":[{"id":"s"}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"block",
            "content":{"tab":"x","pane":{"shadow":"sm"}}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"block",
            "content":{"context":"x"}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"block",
            "content":{"agent":"t3"},"newer_field":[1]}}]}]}),
        // Every field the desktop reads, in its own shape.
        serde_json::from_str(r##"{"spaces":[{"id":"s","tabs":[{"id":"t","root":{"id":"r","type":"stack",
            "active":"b","header":{"content":{"mod":"tabs","props":{}},"reveal":"hover","height":28},
            "cloned_from":{"name":"x","version":"1"},"size":{"weight":2},"min":10,"max":900.5,
            "hidden":false,"title":"T","background":{"gradient":["#000","#fff"]},
            "border":{"width":2},"radius":4,"shadow":"sm","padding":[2,4],
            "focus":{"border":{"color":"accent"},"background":"bg"},"theme":{"light":"a","dark":"b"},
            "colors":{"bg":"#111"},"font":"Mono","font_size":13,"font_weight":300,
            "terminal_line_height":1.2,"terminal_cell_width":1,"terminal_scroll_speed":0.5,"ui_font":"Sans",
            "terminal_padding":[2,2],"terminal_opacity":0.9,"gap":2,"divider":{},
            "movable":true,"closable":false,"resizable":true,"titlebar":"none",
            "traffic_lights":[8,8],"preview":"p","terminal_menu":"m","link_handlers":["open"],
            "toast":"t","window_drag":true,"window_blur":true,
            "children":[{"id":"b","type":"block","size":{"px":200},"padding":3,
                "background":{"image":"bg.png","fit":"contain"},
                "content":{"tab":"active","pane":{"header":{"content":"tabs"},
                    "background":"#222","border":{},"focus":{"width":1},"radius":2,"gap":1,"lone":true}}}],
            "floating":[{"id":"f","content":{"mod":"clock"},"edge":"top","align":"end","peek":4,"hidden":true}]}}]}]}"##).unwrap(),
    ] {
        assert_eq!(check_writable(&ok), Ok(()), "{ok}");
    }
}

#[test]
fn shapes_the_app_cannot_read_are_not_writable() {
    let bad = [
        json!([]),
        json!({"version":2,"workspaces":[]}),
        json!({"spaces":3}),
        json!({"spaces":[{"name":"x"}]}),
        json!({"spaces":[{"id":""}]}),
        json!({"spaces":[{"id":"s"},{"id":"s"}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t"}]},{"id":"r","tabs":[{"id":"t"}]}]}),
        json!({"spaces":[{"id":"s","name":3}]}),
        json!({"spaces":[{"id":"s","tabs":{}}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":null}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"id":"r"}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"future"}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row"}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[
            {"type":"column","children":[{"type":"block"}]}]}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"block",
            "content":{"video":"x"}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"block",
            "content":{"terminal":"t1","mod":"files"}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"block",
            "content":{"terminal":7}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "floating":[{"content":{"terminal":"t1"}}]}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "floating":[{"id":"f","content":{"terminal":"t1"},"edge":"middle"}]}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "floating":[{"id":"f","content":{"terminal":"t1"},"hidden":"yes"}]}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "floating":{}}}]}]}),
        // Fields the desktop knows, of the wrong type.
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "window_drag":"yes"}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "font_weight":"bold"}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "font_weight":70000}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "terminal_line_height":"tall"}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "link_handlers":"open"}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "window_blur":1}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"block",
            "content":{"tab":"active","pane":{"lone":"yes"}}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "title":3}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "size":{"em":2}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "border":{"width":"thin"}}}]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","root":{"type":"row","children":[],
            "titlebar":"custom"}}]}]}),
        json!({"spaces":[{"id":"s","folder":["/tmp"]}]}),
        json!({"spaces":[{"id":"s","tabs":[{"id":"t","floating":[{"rect":[0,0,1,1]}]}]}]}),
    ];
    for b in bad {
        assert!(check_writable(&b).is_err(), "{b}");
    }
}

#[test]
fn a_new_space_is_built_like_the_apps() {
    // herdr-app a547b16 `add_space` + `Tab::new`, serialized through
    // `mirror::Document` (no optional node fields set).
    let golden = json!({"id":"spX","name":"web","folder":"/tmp/w","tabs":[
        {"id":"tbX","name":"Shell","root":{"id":"tbX","type":"row","children":[
            {"id":"pX","type":"block","content":{"terminal":"t7"}}]}}]});
    let mut d = Value::Null;
    let changed = place(
        &mut d,
        &Placing {
            space: "spX",
            new_space: Some(("web", "/tmp/w")),
            tab: "tbX",
            tab_name: "Shell",
            pane: "pX",
            address: "t7",
        },
    )
    .unwrap();
    assert!(changed);
    assert_eq!(d, json!({"spaces":[golden]}));
    assert_eq!(check_writable(&d), Ok(()));
}

#[test]
fn a_new_tab_goes_to_the_end_and_keeps_everything_else() {
    let mut d = doc();
    let p = Placing {
        space: "sa",
        new_space: None,
        tab: "tN",
        tab_name: "new",
        pane: "pN",
        address: "t8",
    };
    assert_eq!(place(&mut d, &p), Ok(true));
    assert_eq!(tab_ids(&d, "sa").unwrap(), ["ta1", "ta2", "tN"]);
    assert_eq!(d["future"], json!({"kept":true}));
    assert_eq!(d["spaces"][0]["color"], "teal");
    assert_eq!(d["spaces"][0]["tabs"][0]["zoomed"], "p1");
    // Applying it again changes nothing.
    assert_eq!(place(&mut d, &p), Ok(false));
    assert_eq!(placement_of(&d, "t8"), Some(("sa".into(), "tN".into())));
}

#[test]
fn a_terminal_already_placed_gets_no_second_pane() {
    let mut d = doc();
    // Someone else put t8 into tab ta2 already (with their own pane id).
    let block = json!({"id":"pOther","type":"block","content":{"terminal":"t8"}});
    d["spaces"][0]["tabs"][1]["root"]["children"]
        .as_array_mut()
        .unwrap()
        .push(block);
    let before = d.clone();
    let p = Placing {
        space: "sa",
        new_space: None,
        tab: "tN",
        tab_name: "new",
        pane: "pN",
        address: "t8",
    };
    assert_eq!(place(&mut d, &p), Ok(false));
    assert_eq!(d, before);
}

#[test]
fn an_adopted_unnamed_space_and_tab_get_our_names_and_stay_in_place() {
    // The desktop adopted the terminal first: unnamed space, unnamed tab.
    let mut d = json!({"spaces":[
        {"id":"sOld","name":"old","folder":"/","tabs":[{"id":"tO","name":"o"}]},
        {"id":"spX","name":"","folder":"","tabs":[
            {"id":"tbX","name":"","root":{"id":"tbX","type":"row","children":[
                {"id":"pDesk","type":"block","content":{"terminal":"t7"}}]}}]},
        {"id":"sLast","name":"last","folder":"/","tabs":[{"id":"tL","name":"l"}]}]});
    let p = Placing {
        space: "spX",
        new_space: Some(("web", "/tmp/w")),
        tab: "tbX",
        tab_name: "Shell",
        pane: "pMine",
        address: "t7",
    };
    assert_eq!(place(&mut d, &p), Ok(true));
    assert_eq!(space_ids(&d), ["sOld", "spX", "sLast"]);
    assert_eq!(d["spaces"][1]["name"], "web");
    assert_eq!(d["spaces"][1]["folder"], "/tmp/w");
    assert_eq!(d["spaces"][1]["tabs"][0]["name"], "Shell");
    assert_eq!(tab_image(&d, "tbX").unwrap().nodes.len(), 2);
    // A name someone gave in the meantime stays.
    d["spaces"][1]["name"] = json!("renamed");
    assert_eq!(place(&mut d, &p), Ok(false));
    assert_eq!(d["spaces"][1]["name"], "renamed");
}

#[test]
fn a_space_deleted_meanwhile_is_not_recreated() {
    let mut d = doc();
    let p = Placing {
        space: "sGone",
        new_space: None,
        tab: "tN",
        tab_name: "",
        pane: "pN",
        address: "t8",
    };
    assert_eq!(place(&mut d, &p), Err(Gone));
}

#[test]
fn a_pane_joins_an_existing_tab() {
    let mut d = json!({"spaces":[{"id":"s","tabs":[
        {"id":"t","root":{"id":"b","type":"block","content":{"terminal":"t1"}}},
        {"id":"u"}]}]});
    for (tab, address) in [("t", "t2"), ("u", "t3")] {
        let p = Placing {
            space: "s",
            new_space: None,
            tab,
            tab_name: "",
            pane: "pN",
            address,
        };
        assert_eq!(place(&mut d, &p), Ok(true));
    }
    assert_eq!(placement_of(&d, "t2"), Some(("s".into(), "t".into())));
    assert_eq!(placement_of(&d, "t3"), Some(("s".into(), "u".into())));
    assert_eq!(check_writable(&d), Ok(()));
}

#[test]
fn renames() {
    let mut d = doc();
    assert_eq!(rename_space(&mut d, "sb", "beta"), Ok(true));
    assert_eq!(rename_space(&mut d, "sb", "beta"), Ok(false));
    assert_eq!(rename_tab(&mut d, "ta2", "zwei"), Ok(true));
    assert_eq!(d["spaces"][0]["tabs"][1]["name"], "zwei");
    assert_eq!(rename_tab(&mut d, "nope", "x"), Err(Gone));
    assert_eq!(rename_space(&mut d, "nope", "x"), Err(Gone));
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn moving_uses_the_boundary_before_the_move() {
    let list = ids(&["a", "b", "c", "d"]);
    let cases = [
        // (item, from, boundary) → order
        ("a", 0, 2, ["b", "a", "c", "d"]), // one step down: from + 2
        ("b", 1, 0, ["b", "a", "c", "d"]), // to the top
        ("a", 0, 4, ["b", "c", "d", "a"]), // to the end
        ("d", 3, 2, ["a", "b", "d", "c"]), // one step up
        ("c", 2, 2, ["a", "b", "c", "d"]), // in place
        ("c", 2, 3, ["a", "b", "c", "d"]), // in place, boundary after it
    ];
    for (item, _from, boundary, want) in cases {
        let mut d = json!({"spaces": list.iter().map(|id| json!({"id":id})).collect::<Vec<_>>()});
        let anchor = Anchor::from_index(&list, boundary);
        move_space(&mut d, item, &anchor).unwrap();
        assert_eq!(space_ids(&d), want, "{item} to {boundary}");
    }
}

#[test]
fn moving_against_a_changed_list_uses_the_anchor() {
    let mut d = json!({"spaces":[{"id":"a"},{"id":"x"},{"id":"b"},{"id":"c"}]});
    // Seen before x was inserted: a goes before c.
    let anchor = Anchor::from_index(&ids(&["a", "b", "c"]), 2);
    assert_eq!(move_space(&mut d, "a", &anchor), Ok(true));
    assert_eq!(space_ids(&d), ["x", "b", "a", "c"]);
    // Again: nothing changes.
    assert_eq!(move_space(&mut d, "a", &anchor), Ok(false));
    // An anchor that is gone: where it was meant to go is no longer clear,
    // so nothing moves (the end could be the opposite direction).
    assert_eq!(
        move_space(&mut d, "x", &Anchor::Before("gone".into())),
        Err(NotMoved::AnchorGone)
    );
    assert_eq!(space_ids(&d), ["x", "b", "a", "c"]);
    assert_eq!(
        move_space(&mut d, "nope", &Anchor::End),
        Err(NotMoved::Gone)
    );
}

#[test]
fn tabs_move_within_their_space() {
    let mut d = doc();
    let anchor = Anchor::from_index(&tab_ids(&d, "sa").unwrap(), 0);
    assert_eq!(move_tab(&mut d, "ta2", &anchor), Ok(true));
    assert_eq!(tab_ids(&d, "sa").unwrap(), ["ta2", "ta1"]);
    assert_eq!(tab_ids(&d, "sb").unwrap(), ["tb1"]);
}

#[test]
fn images_hold_nodes_addresses_and_floating_blocks() {
    let d = doc();
    let t = tab_image(&d, "ta1").unwrap();
    assert_eq!(
        t.addresses,
        ["t1", "t2", "t9"].iter().map(|s| s.to_string()).collect()
    );
    assert!(t.nodes.contains("p3") && t.nodes.contains("f1"));
    let s = space_image(&d, "sa").unwrap();
    assert!(s.addresses.contains("t4") && s.tabs.contains("ta2"));
    assert!(!s.grew_since(&s));
    let mut more = d.clone();
    more["spaces"][0]["tabs"][0]["root"]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"pMod","type":"block","content":{"mod":"notes"}}));
    assert!(tab_image(&more, "ta1").unwrap().grew_since(&t));
    // A tab-level floating pane in a newer build counts too (herdr-app
    // `mirror.rs` test shape: `{"pane": node, "rect": [..]}`).
    let mut float = d.clone();
    float["spaces"][1]["tabs"][0]["floating"] = json!([{"pane":
        {"id":"fT","type":"block","content":{"terminal":"t6"}},"rect":[0.2,0.2,0.6,0.6]}]);
    assert_eq!(check_writable(&float), Ok(()));
    let image = tab_image(&float, "tb1").unwrap();
    assert!(image.addresses.contains("t6") && image.nodes.contains("fT"));
    assert_eq!(
        placement_of(&float, "t6"),
        Some(("sb".into(), "tb1".into()))
    );
    assert_eq!(placement_of(&d, "t9"), Some(("sa".into(), "ta1".into())));
    assert_eq!(placement_of(&d, "t0"), None);
}

#[test]
fn removing_a_last_tab_removes_its_space() {
    let mut d = doc();
    assert!(remove_tab(&mut d, "ta2"));
    assert_eq!(tab_ids(&d, "sa").unwrap(), ["ta1"]);
    assert!(remove_tab(&mut d, "tb1"));
    assert_eq!(space_ids(&d), ["sa"]);
    assert!(!remove_tab(&mut d, "tb1"));
    assert!(remove_space(&mut d, "sa"));
    assert!(!remove_space(&mut d, "sa"));
    assert_eq!(d, json!({"spaces":[],"future":{"kept":true}}));
}
