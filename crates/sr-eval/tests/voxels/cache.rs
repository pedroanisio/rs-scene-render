//! The products derived from the cells of a body, kept by the content of the cells: asked again with the same content they cost nothing, and with
//! other content exactly that body is worked out again.

use sr_3d::occupancy::Occupancy;
use sr_eval::voxel_cache::DerivedCache;
use sr_eval::voxels::body;
use std::sync::Arc;

const SIZE: [f64; 3] = [0.25; 3];
const DENSITY: f64 = 2400.0;

fn block(nx: i32, ny: i32, nz: i32) -> Occupancy {
    let mut cells = Vec::new();
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                cells.push(([i, j, k], 1u8));
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

const BIG: usize = 1 << 30;

#[test]
fn the_same_content_asked_twice_is_worked_out_once_and_is_the_same_value() {
    let mut cache = DerivedCache::new(BIG);
    let o = block(6, 5, 4);
    let a = cache.properties(&o, SIZE, DENSITY).unwrap();
    let b = cache.properties(&o, SIZE, DENSITY).unwrap();
    assert_eq!((cache.computed(), cache.hits()), (1, 1));
    assert!(Arc::ptr_eq(&a, &b));
    // and it is what the occupancy says of itself, to the bit
    assert_eq!(*a, o.moments().properties(SIZE, DENSITY).unwrap());
    let c1 = cache.components(&o);
    let c2 = cache.components(&o);
    assert!(Arc::ptr_eq(&c1, &c2));
    assert_eq!(*c1, o.components());
    let w1 = cache.body(&o, SIZE, DENSITY, 1.0).unwrap();
    let w2 = cache.body(&o, SIZE, DENSITY, 1.0).unwrap();
    assert!(Arc::ptr_eq(&w1, &w2));
    assert_eq!(*w1, body(&o, SIZE, DENSITY, 1.0).unwrap());
    assert_eq!((cache.computed(), cache.hits()), (3, 3), "one of each product, asked twice");
}

#[test]
fn a_change_to_one_body_is_worked_out_again_for_that_body_and_for_no_other() {
    let mut cache = DerivedCache::new(BIG);
    let (mut a, b) = (block(6, 5, 4), block(3, 3, 3));
    let (pa, pb) = (cache.properties(&a, SIZE, DENSITY).unwrap(), cache.properties(&b, SIZE, DENSITY).unwrap());
    assert_eq!(cache.computed(), 2);
    a.set([0, 0, 0], 0).unwrap();
    let pa2 = cache.properties(&a, SIZE, DENSITY).unwrap();
    assert_eq!(cache.computed(), 3, "exactly the body that changed");
    assert!(!Arc::ptr_eq(&pa, &pa2) && pa.mass != pa2.mass);
    let pb2 = cache.properties(&b, SIZE, DENSITY).unwrap();
    assert_eq!(cache.computed(), 3, "the other body is as it was");
    assert!(Arc::ptr_eq(&pb, &pb2));
}

#[test]
fn the_same_content_made_another_way_is_the_same_entry() {
    // the revision of a grid says nothing across grids: the content is what is asked
    let mut cache = DerivedCache::new(BIG);
    let a = block(5, 4, 3);
    let mut cells: Vec<([i32; 3], u8)> = a.cells().map(|c| (c, 1)).collect();
    cells.reverse();
    let mut b = Occupancy::from_cells(cells.into_iter().take(10)).unwrap();
    let missing: Vec<[i32; 3]> = a.cells().filter(|c| b.get(*c) == 0).collect();
    for c in missing {
        b.set(c, 1).unwrap();
    }
    b.set([0, 0, 0], 0).unwrap();
    b.set([0, 0, 0], 1).unwrap();
    assert_ne!(a.revision(), b.revision());
    let pa = cache.properties(&a, SIZE, DENSITY).unwrap();
    let pb = cache.properties(&b, SIZE, DENSITY).unwrap();
    assert_eq!((cache.computed(), cache.hits()), (1, 1));
    assert!(Arc::ptr_eq(&pa, &pb));
}

#[test]
fn going_back_to_an_earlier_content_finds_it_while_it_is_kept_and_works_it_out_again_to_the_same_bits_when_it_is_not() {
    let mut a = block(6, 5, 4);
    let size = SIZE;
    let first = a.moments().properties(size, DENSITY).unwrap();
    for budget in [BIG, 0] {
        let mut cache = DerivedCache::new(budget);
        let p1 = cache.properties(&a, size, DENSITY).unwrap();
        a.set([5, 4, 3], 0).unwrap();
        let _ = cache.properties(&a, size, DENSITY).unwrap();
        a.set([5, 4, 3], 1).unwrap();
        let again = cache.properties(&a, size, DENSITY).unwrap();
        assert_eq!(*again, first, "the same bits");
        if budget == BIG {
            assert_eq!(cache.computed(), 2);
            assert!(Arc::ptr_eq(&p1, &again), "the seek back finds the entry");
        } else {
            assert_eq!(
                (cache.computed(), cache.hits(), cache.bytes()),
                (3, 0, 0),
                "nothing is kept, everything is right"
            );
        }
    }
}

#[test]
fn another_cell_size_density_or_scale_is_another_entry() {
    let mut cache = DerivedCache::new(BIG);
    let o = block(4, 4, 4);
    cache.properties(&o, SIZE, DENSITY).unwrap();
    cache.properties(&o, [0.5; 3], DENSITY).unwrap();
    cache.properties(&o, SIZE, 1000.0).unwrap();
    assert_eq!((cache.computed(), cache.hits()), (3, 0));
    cache.body(&o, SIZE, DENSITY, 1.0).unwrap();
    cache.body(&o, SIZE, DENSITY, 100.0).unwrap();
    assert_eq!((cache.computed(), cache.hits()), (5, 0), "a scale is part of the body's key");
    cache.body(&o, SIZE, DENSITY, 100.0).unwrap();
    assert_eq!(cache.hits(), 1);
}

#[test]
fn the_budget_is_never_passed_and_what_is_dropped_is_the_least_recently_asked() {
    let (a, b, c) = (block(8, 8, 8), block(9, 8, 8), block(10, 8, 8));
    let bytes_of_one = {
        let mut probe = DerivedCache::new(BIG);
        probe.body(&a, SIZE, DENSITY, 1.0).unwrap();
        probe.bytes()
    };
    assert!(bytes_of_one > 8 * 8 * 8 * 12, "a body is charged for its cells: {bytes_of_one}");
    // room for two bodies of this size and not three
    let budget = 2 * bytes_of_one + bytes_of_one / 2;
    let run = || {
        let mut cache = DerivedCache::new(budget);
        let mut trace = Vec::new();
        for o in [&a, &b, &a, &c, &a, &b] {
            cache.body(o, SIZE, DENSITY, 1.0).unwrap();
            assert!(cache.bytes() <= budget, "{} over {budget}", cache.bytes());
            trace.push((cache.computed(), cache.hits(), cache.evicted(), cache.bytes()));
        }
        trace
    };
    let first = run();
    // a, b, then a again is a hit (b is the older), c pushes b out, a is a hit, b is worked out again
    let flags: Vec<(u64, u64)> = first.iter().map(|t| (t.0, t.1)).collect();
    assert_eq!(flags, vec![(1, 0), (2, 0), (2, 1), (3, 1), (3, 2), (4, 2)]);
    assert!(first.last().unwrap().2 >= 2);
    // the same requests give the same counters, the same bytes: no clock, no address
    assert_eq!(run(), first);
}

#[test]
fn what_cannot_be_derived_is_not_kept_and_says_what_the_function_says() {
    let mut cache = DerivedCache::new(BIG);
    let empty = Occupancy::from_cells(Vec::<([i32; 3], u8)>::new()).unwrap();
    let wanted = body(&empty, SIZE, DENSITY, 1.0).unwrap_err();
    assert_eq!(cache.body(&empty, SIZE, DENSITY, 1.0).unwrap_err(), wanted);
    assert_eq!(cache.body(&empty, SIZE, DENSITY, 1.0).unwrap_err(), wanted);
    let o = block(2, 2, 2);
    let invalid = o.moments().properties([0.0; 3], DENSITY).unwrap_err();
    assert_eq!(cache.properties(&o, [0.0; 3], DENSITY).unwrap_err(), invalid);
    assert_eq!((cache.computed(), cache.hits(), cache.bytes()), (0, 0, 0));
}

#[test]
fn two_contents_that_have_the_same_fingerprint_do_not_share_a_product() {
    // a fingerprint that is the same for every grid: the key cannot tell the contents apart (the count and the bricks are the same too), and only the
    // comparison of the content does
    let mut cache = DerivedCache::with_fingerprint(BIG, |_| 7);
    let a = block(6, 4, 4);
    let mut b = block(6, 4, 4);
    b.set([0, 0, 0], 0).unwrap();
    b.set([6, 0, 0], 1).unwrap();
    assert_eq!(
        (a.count(), a.bricks().count()),
        (b.count(), b.bricks().count()),
        "the same count and bricks, other content"
    );
    let pa = cache.properties(&a, SIZE, DENSITY).unwrap();
    let pb = cache.properties(&b, SIZE, DENSITY).unwrap();
    assert_eq!((cache.computed(), cache.hits()), (2, 0));
    assert_ne!(*pa, *pb);
    assert_eq!(*pb, b.moments().properties(SIZE, DENSITY).unwrap());
    // and each is found again by its own content
    assert!(Arc::ptr_eq(&cache.properties(&a, SIZE, DENSITY).unwrap(), &pa));
    assert!(Arc::ptr_eq(&cache.properties(&b, SIZE, DENSITY).unwrap(), &pb));
    assert_eq!((cache.computed(), cache.hits()), (2, 2));
}

#[test]
fn two_caches_that_are_asked_alternately_keep_nothing_in_common() {
    // as the programs of a process do: the same requests in two caches, interleaved, give each what it would have had alone
    let (a, b, c) = (block(8, 8, 8), block(9, 8, 8), block(10, 8, 8));
    let one = {
        let mut probe = DerivedCache::new(BIG);
        probe.body(&a, SIZE, DENSITY, 1.0).unwrap();
        probe.bytes()
    };
    let budget = 2 * one + one / 2;
    let requests = |first: bool| -> Vec<&Occupancy> {
        if first {
            vec![&a, &b, &a, &c, &a]
        } else {
            vec![&c, &c, &b, &a, &b]
        }
    };
    let alone = |first: bool| {
        let mut cache = DerivedCache::new(budget);
        for o in requests(first) {
            cache.body(o, SIZE, DENSITY, 1.0).unwrap();
        }
        (cache.computed(), cache.hits(), cache.evicted(), cache.bytes())
    };
    let (mut x, mut y) = (DerivedCache::new(budget), DerivedCache::new(budget));
    for (p, q) in requests(true).into_iter().zip(requests(false)) {
        x.body(p, SIZE, DENSITY, 1.0).unwrap();
        y.body(q, SIZE, DENSITY, 1.0).unwrap();
    }
    assert_eq!((x.computed(), x.hits(), x.evicted(), x.bytes()), alone(true));
    assert_eq!((y.computed(), y.hits(), y.evicted(), y.bytes()), alone(false));
    assert_ne!(alone(true), alone(false), "the two sequences are not the same one");
}

#[test]
fn the_components_and_the_body_do_not_share_across_a_fingerprint_collision_either() {
    let mut cache = DerivedCache::with_fingerprint(BIG, |_| 7);
    // two separate blocks, and the same count and bricks with one cell moved so that they touch
    let mut a = block(3, 3, 3);
    for c in block(3, 3, 3).cells().map(|c| [c[0] + 4, c[1], c[2]]) {
        a.set(c, 1).unwrap();
    }
    let mut b = a.clone();
    b.set([6, 0, 0], 0).unwrap();
    b.set([3, 0, 0], 1).unwrap();
    assert_eq!((a.count(), a.bricks().count()), (b.count(), b.bricks().count()));
    let (ca, cb) = (cache.components(&a), cache.components(&b));
    assert_eq!((cache.computed(), cache.hits()), (2, 0));
    assert_eq!(*ca, a.components());
    assert_eq!(*cb, b.components());
    assert_ne!(ca.len(), cb.len(), "one touches and the other does not");
    let (wa, wb) = (cache.body(&a, SIZE, DENSITY, 1.0).unwrap(), cache.body(&b, SIZE, DENSITY, 1.0).unwrap());
    assert_eq!(*wa, body(&a, SIZE, DENSITY, 1.0).unwrap());
    assert_eq!(*wb, body(&b, SIZE, DENSITY, 1.0).unwrap());
    assert_ne!(*wa, *wb);
    // and each is found by its own content
    assert!(Arc::ptr_eq(&cache.components(&a), &ca) && Arc::ptr_eq(&cache.body(&b, SIZE, DENSITY, 1.0).unwrap(), &wb));
}

#[test]
fn a_product_that_cannot_be_kept_is_not_copied_and_what_is_kept_is_charged_by_its_room() {
    let o = block(8, 8, 8);
    let mut cache = DerivedCache::new(BIG);
    cache.body(&o, SIZE, DENSITY, 1.0).unwrap();
    let kept = cache.bytes();
    // the cells at twelve bytes and the brick at 512 and its key: not less than that, and not much more
    let wanted = 8 * 8 * 8 * 12 + 128 + (512 + 24);
    assert!(kept >= wanted && kept <= wanted + 64, "{kept} bytes for {wanted}");
    // a budget that is one byte short of it keeps nothing
    let mut short = DerivedCache::new(kept - 1);
    short.body(&o, SIZE, DENSITY, 1.0).unwrap();
    assert_eq!((short.bytes(), short.evicted()), (0, 0));
}

#[test]
fn the_brick_that_the_world_counts_bricks_in_is_the_brick_of_an_occupancy() {
    // sr-sim does not depend on sr-3d and says the side of a brick itself: the two must not part
    assert_eq!(sr_sim::physics3d::VOXEL_BRICK, sr_3d::occupancy::BRICK);
}
