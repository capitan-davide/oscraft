/*
 * The layout of QNX's startup structures, as the QNX SDP headers define
 * them, for checking the Rust mirrors in src/abi/.
 *
 * Each entry declares an array whose size is the value to record plus one
 * (so that no array is empty); layout.py reads the sizes back from the
 * compiled object with nm, so nothing has to run on the target. Entries
 * are named after the Rust types and fields.
 */
#include <stddef.h>
#include <sys/elf.h>
#include <sys/image.h>
#include <sys/startup.h>
#include <sys/syspage.h>

// ANCHOR: layout_macros
#define SIZE(rust, c)          char abi__##rust##__size[sizeof(c) + 1];
#define OFF(rust, rf, c, cf)   char abi__##rust##__##rf[offsetof(c, cf) + 1];
/* A structure ending in a variable-sized array (__FLEXARY): the Rust type
 * stops where the array starts, so its size is the array's offset. */
#define FLEXSIZE(rust, c, cf)  char abi__##rust##__size[offsetof(c, cf) + 1];
// ANCHOR_END: layout_macros

SIZE(StartupHeader, struct startup_header)
OFF(StartupHeader, image_paddr, struct startup_header, image_paddr)
OFF(StartupHeader, startup_size, struct startup_header, startup_size)
OFF(StartupHeader, imagefs_size, struct startup_header, imagefs_size)
OFF(StartupHeader, addr_off, struct startup_header, addr_off)
OFF(StartupHeader, info, struct startup_header, info)

SIZE(BootArgsEntry, struct bootargs_entry)
OFF(BootArgsEntry, shdr_addr, struct bootargs_entry, shdr_addr)

SIZE(SyspageEntry, struct syspage_entry)
OFF(SyspageEntry, system_private, struct syspage_entry, system_private)
OFF(SyspageEntry, version_major, struct syspage_entry, version.major)
OFF(SyspageEntry, hwinfo, struct syspage_entry, hwinfo)
OFF(SyspageEntry, qtime, struct syspage_entry, qtime)
OFF(SyspageEntry, callout, struct syspage_entry, callout)
OFF(SyspageEntry, typed_strings, struct syspage_entry, typed_strings)
OFF(SyspageEntry, strings, struct syspage_entry, strings)
OFF(SyspageEntry, smp, struct syspage_entry, smp)
OFF(SyspageEntry, un, struct syspage_entry, un)
OFF(SyspageEntry, asinfo, struct syspage_entry, asinfo)
OFF(SyspageEntry, cpuinfo, struct syspage_entry, cpuinfo)
OFF(SyspageEntry, cacheattr, struct syspage_entry, cacheattr)
OFF(SyspageEntry, intrinfo, struct syspage_entry, intrinfo)
OFF(SyspageEntry, hypinfo, struct syspage_entry, hypinfo)
OFF(SyspageEntry, cluster, struct syspage_entry, cluster)

SIZE(Aarch64SyspageEntry, struct aarch64_syspage_entry)
OFF(Aarch64SyspageEntry, gic_map, struct aarch64_syspage_entry, gic_map)
OFF(Aarch64SyspageEntry, gicr_map, struct aarch64_syspage_entry, gicr_map)
OFF(Aarch64SyspageEntry, idreg_dump, struct aarch64_syspage_entry, idreg_dump)
OFF(Aarch64SyspageEntry, rsvd, struct aarch64_syspage_entry, rsvd)

SIZE(SystemPrivateEntry, struct system_private_entry)
OFF(SystemPrivateEntry, kdump_info, struct system_private_entry, kdump_info)
OFF(SystemPrivateEntry, user_cpupageptr, struct system_private_entry, user_cpupageptr)
OFF(SystemPrivateEntry, kern_syspageptr, struct system_private_entry, kern_syspageptr)
OFF(SystemPrivateEntry, entropy, struct system_private_entry, entropy)

SIZE(QtimeEntry, struct qtime_entry)
OFF(QtimeEntry, nsec_inc, struct qtime_entry, nsec_inc)
OFF(QtimeEntry, adjust_tick_count, struct qtime_entry, adjust.tick_count)
OFF(QtimeEntry, timer_period, struct qtime_entry, timer_period)
OFF(QtimeEntry, intr, struct qtime_entry, intr)
OFF(QtimeEntry, flags, struct qtime_entry, flags)
OFF(QtimeEntry, timer_load_max, struct qtime_entry, timer_load_max)
OFF(QtimeEntry, tick_period_cc, struct qtime_entry, tick_period_cc)

SIZE(CalloutEntry, struct callout_entry)
OFF(CalloutEntry, debug, struct callout_entry, debug)
OFF(CalloutEntry, debug_watchdog, struct callout_entry, debug_watchdog)
OFF(CalloutEntry, custom, struct callout_entry, custom)
SIZE(DebugCallout, struct debug_callout)

SIZE(AsinfoEntry, struct asinfo_entry)
OFF(AsinfoEntry, owner, struct asinfo_entry, owner)
OFF(AsinfoEntry, priority, struct asinfo_entry, priority)

SIZE(CpuinfoEntry, struct cpuinfo_entry)
OFF(CpuinfoEntry, flags, struct cpuinfo_entry, flags)
OFF(CpuinfoEntry, smp_hwcoreid, struct cpuinfo_entry, smp_hwcoreid)
OFF(CpuinfoEntry, name, struct cpuinfo_entry, name)
OFF(CpuinfoEntry, data_cache, struct cpuinfo_entry, data_cache)

SIZE(CacheattrEntry, struct cacheattr_entry)
OFF(CacheattrEntry, ways, struct cacheattr_entry, ways)
OFF(CacheattrEntry, control, struct cacheattr_entry, control)

SIZE(IntrgenData, struct __intrgen_data)
OFF(IntrgenData, rtn, struct __intrgen_data, rtn)
SIZE(IntrinfoEntry, struct intrinfo_entry)
OFF(IntrinfoEntry, cpu_intr_stride, struct intrinfo_entry, cpu_intr_stride)
OFF(IntrinfoEntry, id, struct intrinfo_entry, id)
OFF(IntrinfoEntry, eoi, struct intrinfo_entry, eoi)
OFF(IntrinfoEntry, mask, struct intrinfo_entry, mask)
OFF(IntrinfoEntry, config, struct intrinfo_entry, config)
OFF(IntrinfoEntry, local_stride, struct intrinfo_entry, local_stride)

SIZE(SmpEntry, struct smp_entry)
OFF(SmpEntry, send_ipi, struct smp_entry, send_ipi)
OFF(SmpEntry, start_addr, struct smp_entry, start_addr)
SIZE(HypinfoEntry, struct hypinfo_entry)
SIZE(ClusterEntry, struct cluster_entry)

SIZE(CpupageEntry, struct cpupage_entry)
OFF(CpupageEntry, un, struct cpupage_entry, un)
OFF(CpupageEntry, pls, struct cpupage_entry, pls)

FLEXSIZE(ImageHeader, struct image_header, mountpoint)
OFF(ImageHeader, dir_offset, struct image_header, dir_offset)
OFF(ImageHeader, mountflags, struct image_header, mountflags)
SIZE(ImageAttr, struct image_attr)
FLEXSIZE(ImageFile, struct image_file, path)
OFF(ImageFile, offset, struct image_file, offset)
SIZE(IfsBootstrapHead, struct ifs_bootstrap_head)
OFF(IfsBootstrapHead, bootstrap, struct ifs_bootstrap_head, bootstrap)
SIZE(IfsBootstrapData, struct ifs_bootstrap_data)
OFF(IfsBootstrapData, next_entry, struct ifs_bootstrap_data, next_entry)
OFF(IfsBootstrapData, args, struct ifs_bootstrap_data, args)

SIZE(Elf64Ehdr, Elf64_Ehdr)
OFF(Elf64Ehdr, e_entry, Elf64_Ehdr, e_entry)
OFF(Elf64Ehdr, e_phnum, Elf64_Ehdr, e_phnum)
SIZE(Elf64Phdr, Elf64_Phdr)
OFF(Elf64Phdr, p_vaddr, Elf64_Phdr, p_vaddr)
OFF(Elf64Phdr, p_memsz, Elf64_Phdr, p_memsz)
