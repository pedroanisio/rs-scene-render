// SREP 67 kit: 1000 points on (100, 100), 100 on (200, 100), 10 on (300, 100).
fn sr_point(i: u32) {
  if (i < 1000u) { sr_accumulate(100, 100, 0u, 1u); }
  else if (i < 1100u) { sr_accumulate(200, 100, 0u, 1u); }
  else { sr_accumulate(300, 100, 0u, 1u); }
}
