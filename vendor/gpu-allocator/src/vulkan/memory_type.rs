//! CPU upload memory selection, separated from Vulkan handles for behavioral tests.

/// `(index, property flags)` in Vulkan order. Honor the resource's compatibility mask.
pub(crate) fn upload_type(
    types: impl Iterator<Item = (u32, u32)>,
    compatible: u32,
    required: u32,
    device_local: u32,
    skip: Option<u32>,
) -> Option<u32> {
    let mut fallback = None;
    for (i, flags) in types {
        if Some(i) == skip || compatible & (1 << i) == 0 || flags & required != required { continue; }
        if flags & device_local == 0 { return Some(i); }
        fallback.get_or_insert(i);
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    const LOCAL: u32 = 1;
    const VISIBLE: u32 = 2;
    const COHERENT: u32 = 4;
    const CACHED: u32 = 8;
    const HOST: u32 = VISIBLE | COHERENT;

    #[test]
    fn prefers_nonlocal_even_when_vulkan_legally_lists_bar_memory_first() {
        // Neither flag set is a subset of the other: either ordering is legal in Vulkan.
        let types = [(0, LOCAL | HOST), (1, HOST | CACHED)];
        assert_eq!(upload_type(types.into_iter(), 3, HOST, LOCAL, None), Some(1));
    }
    #[test]
    fn uma_and_resource_masks_keep_the_compatible_fallback() {
        assert_eq!(upload_type([(0, LOCAL | HOST)].into_iter(), 1, HOST, LOCAL, None), Some(0));
        let types = [(0, LOCAL | HOST), (1, HOST | CACHED), (2, VISIBLE)];
        assert_eq!(upload_type(types.into_iter(), 1, HOST, LOCAL, None), Some(0));
        assert_eq!(upload_type(types.into_iter(), 4, HOST, LOCAL, None), None);
        assert_eq!(upload_type(types.into_iter(), 3, HOST, LOCAL, Some(1)), Some(0));
        assert_eq!(upload_type(types.into_iter(), 1, HOST, LOCAL, Some(0)), None);
    }
}
