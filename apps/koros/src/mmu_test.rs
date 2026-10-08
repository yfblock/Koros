//! In-kernel self-check for the `kor-mmu` tool (CortenMM-style).
//!
//! Runs on a scratch address space so the live kernel mappings are never
//! touched.  Exercises the transactional interface end to end — map/query,
//! huge mapping, on-demand split, `mark` (virtual allocation), `unmap`
//! with page-table reclamation — and finishes by proving the tool leaks no
//! frame: the frame allocator's free count returns to its baseline after
//! `destroy`.

use kor::mapping::{MapError, MapSize, MappingFlags};
use kor::println;
use kor_mmu::descriptor::PageStatus;

/// Scratch virtual area: canonical on every supported format (the Sv39 low
/// half tops out at 256 GiB; x86-64/AArch64 at 128 TiB) and far from any
/// kernel or user image.
const SCRATCH_VA: usize = 0x0000_0030_0000_0000;
const TWO_MIB: usize = 2 * 1024 * 1024;

/// Run the kor-mmu self-check; prints `kor-mmu: ... OK` lines.
pub fn run() {
    if kor_mmu::current_format().is_none() {
        println!("kor-mmu: no paging on this architecture, skipped");
        return;
    }

    let baseline = kor::frame::available_frames();
    let space = kor_mmu::AddrSpace::new().expect("root table");

    // Data pages this test allocates (freed again before the leak check).
    let pa_4k = kor::frame::alloc_frame().expect("data page");
    let pa_base = kor::frame::alloc_frames(512).expect("2MiB extent");
    let pa_inner = kor::frame::alloc_frame().expect("inner page");

    // 1. Plain 4 KiB mapping round-trip + duplicate rejection.
    {
        let mut txn = space.lock(SCRATCH_VA..SCRATCH_VA + 4096);
        txn.map(
            SCRATCH_VA,
            pa_4k,
            MappingFlags::R | MappingFlags::W,
            MapSize::Page4K,
        )
        .expect("map 4K");
        match txn.query(SCRATCH_VA) {
            PageStatus::Mapped {
                phys, granularity, ..
            } => {
                assert_eq!(phys, pa_4k);
                assert_eq!(granularity, 4096);
            }
            other => panic!("kor-mmu: query after map = {other:?}"),
        }
        assert_eq!(
            txn.map(SCRATCH_VA, pa_4k, MappingFlags::R, MapSize::Page4K),
            Err(MapError::BlockedByExistingMapping)
        );
    }
    println!("kor-mmu: 4K map/query/dup-rejected OK");

    // 2. Huge (2 MiB) mapping, then split it by mapping a 4K page inside.
    let huge_va = SCRATCH_VA + TWO_MIB;
    {
        let mut txn = space.lock(huge_va..huge_va + TWO_MIB);
        txn.map(
            huge_va,
            pa_base,
            MappingFlags::R | MappingFlags::W,
            MapSize::Page2M,
        )
        .expect("map 2M");
        match txn.query(huge_va) {
            PageStatus::Mapped {
                phys, granularity, ..
            } => {
                assert_eq!(phys, pa_base);
                assert_eq!(granularity, TWO_MIB);
            }
            other => panic!("kor-mmu: query after huge map = {other:?}"),
        }
        // Mapping inside the huge page forces an on-demand split.
        let inner = huge_va + 64 * 1024;
        txn.map(
            inner,
            pa_inner,
            MappingFlags::R | MappingFlags::W,
            MapSize::Page4K,
        )
        .expect("split + map 4K");
        match txn.query(inner) {
            PageStatus::Mapped {
                phys, granularity, ..
            } => {
                assert_eq!(phys, pa_inner);
                assert_eq!(granularity, 4096);
            }
            other => panic!("kor-mmu: query after split = {other:?}"),
        }
        // Siblings of the split remain mapped at 2 MiB granularity.
        match txn.query(huge_va + TWO_MIB / 2) {
            PageStatus::Mapped { .. } => {}
            other => panic!("kor-mmu: sibling query = {other:?}"),
        }
    }
    println!("kor-mmu: 2M map + on-demand split OK");

    // 3. Virtual allocation (on-demand paging bookkeeping, no hardware map).
    let anon_va = SCRATCH_VA + 4 * TWO_MIB;
    {
        let mut txn = space.lock(anon_va..anon_va + TWO_MIB);
        txn.mark(
            anon_va..anon_va + TWO_MIB,
            MappingFlags::R | MappingFlags::W,
        );
        match txn.query(anon_va + 4096) {
            PageStatus::Allocated(flags) => {
                assert!(flags.contains(MappingFlags::R | MappingFlags::W));
            }
            other => panic!("kor-mmu: query after mark = {other:?}"),
        }
    }
    println!("kor-mmu: mark/query (on-demand state) OK");

    // 4. Unmap everything: 512 base pages from the split 2 MiB region, one
    //    from the plain mapping; table pages are reclaimed on the way.
    let unmapped = {
        let mut txn = space.lock(huge_va..huge_va + TWO_MIB);
        txn.unmap(huge_va..huge_va + TWO_MIB).expect("unmap 2M")
    };
    assert_eq!(unmapped, 512);
    {
        let mut txn = space.lock(SCRATCH_VA..SCRATCH_VA + 4096);
        assert_eq!(txn.unmap(SCRATCH_VA..SCRATCH_VA + 4096), Ok(1));
    }
    {
        let txn = space.lock(huge_va..huge_va + 4096);
        assert_eq!(txn.query(huge_va), PageStatus::Invalid);
    }
    println!("kor-mmu: unmap + table reclaim OK");

    // 5. Give the data pages back, tear the address space down, and prove
    //    the frame count is exactly where it started (no table-page leak).
    kor::frame::free_frames(pa_4k, 1);
    kor::frame::free_frames(pa_base, 512);
    kor::frame::free_frames(pa_inner, 1);
    // SAFETY: the scratch address space was never activated.
    unsafe { space.destroy() };
    assert_eq!(kor::frame::available_frames(), baseline);
    println!("kor-mmu: destroy + zero-frame-leak OK");
}
