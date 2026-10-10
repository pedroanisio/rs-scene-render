// SREP 67 kit: the same counts as srep67-three.wgsl (1000, 100, 10), issued in another order.
fn sr_point(i: u32) {
  if (i < 30u) {
    let k = i % 3u;
    if (k == 0u) { sr_accumulate(100, 100, 0u, 1u); }
    else if (k == 1u) { sr_accumulate(200, 100, 0u, 1u); }
    else { sr_accumulate(300, 100, 0u, 1u); }
  } else if (i < 210u) {
    if (i % 2u == 0u) { sr_accumulate(100, 100, 0u, 1u); } else { sr_accumulate(200, 100, 0u, 1u); }
  } else { sr_accumulate(100, 100, 0u, 1u); }
}
