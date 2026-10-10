// SREP 67 kit: channels="4". 50 points on (100, 100) carry red, 50 on (200, 100) carry blue (amounts in 1/256).
fn sr_point(i: u32) {
  if (i < 50u) {
    sr_accumulate(100, 100, 0u, 256u); sr_accumulate(100, 100, 1u, 256u);
  } else {
    sr_accumulate(200, 100, 0u, 256u); sr_accumulate(200, 100, 3u, 256u);
  }
}
