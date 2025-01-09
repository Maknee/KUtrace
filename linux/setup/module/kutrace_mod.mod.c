#include <linux/module.h>
#define INCLUDE_VERMAGIC
#include <linux/build-salt.h>
#include <linux/elfnote-lto.h>
#include <linux/export-internal.h>
#include <linux/vermagic.h>
#include <linux/compiler.h>

#ifdef CONFIG_UNWINDER_ORC
#include <asm/orc_header.h>
ORC_HEADER;
#endif

BUILD_SALT;
BUILD_LTO_INFO;

MODULE_INFO(vermagic, VERMAGIC_STRING);
MODULE_INFO(name, KBUILD_MODNAME);

__visible struct module __this_module
__section(".gnu.linkonce.this_module") = {
	.name = KBUILD_MODNAME,
	.init = init_module,
#ifdef CONFIG_MODULE_UNLOAD
	.exit = cleanup_module,
#endif
	.arch = MODULE_ARCH_INIT,
};

#ifdef CONFIG_RETPOLINE
MODULE_INFO(retpoline, "Y");
#endif



static const struct modversion_info ____versions[]
__used __section("__versions") = {
	{ 0x17de3d5, "nr_cpu_ids" },
	{ 0xb19a5453, "__per_cpu_offset" },
	{ 0x5a5a2271, "__cpu_online_mask" },
	{ 0x53a1e8d9, "_find_next_bit" },
	{ 0x87a21cb3, "__ubsan_handle_out_of_bounds" },
	{ 0x122c3a7e, "_printk" },
	{ 0xf9a482f9, "msleep" },
	{ 0x18161b5e, "kutrace_global_ops" },
	{ 0x999e8297, "vfree" },
	{ 0xd6ee688f, "vmalloc" },
	{ 0xb9876eeb, "kutrace_net_filter" },
	{ 0x34db050b, "_raw_spin_lock_irqsave" },
	{ 0xd35cce70, "_raw_spin_unlock_irqrestore" },
	{ 0xca45d2f, "pcpu_hot" },
	{ 0x53569707, "this_cpu_off" },
	{ 0x48d88a2c, "__SCT__preempt_schedule" },
	{ 0x69acdf38, "memcpy" },
	{ 0xcbd4898c, "fortify_panic" },
	{ 0xf0fdf6cb, "__stack_chk_fail" },
	{ 0x68a12ab8, "rep_movs_alternative" },
	{ 0x6b10bee1, "_copy_to_user" },
	{ 0xc513f1aa, "has_capability" },
	{ 0xfb578fc5, "memset" },
	{ 0x2f99ee32, "param_ops_long" },
	{ 0xbdfb6dbb, "__fentry__" },
	{ 0xb456a31f, "kutrace_tracing" },
	{ 0x5b8239ca, "__x86_return_thunk" },
	{ 0x54b1fac6, "__ubsan_handle_load_invalid_value" },
	{ 0xaae65d69, "kutrace_traceblock_per_cpu" },
	{ 0x8344ac91, "kutrace_pid_filter" },
	{ 0x9f5d4a4e, "module_layout" },
};

MODULE_INFO(depends, "");


MODULE_INFO(srcversion, "2AF3BF43316819B9504B45C");
