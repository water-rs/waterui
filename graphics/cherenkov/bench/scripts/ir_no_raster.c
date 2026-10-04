/* Vulkan layer for the Callgrind Ir gate: make every command that does
 * per-pixel or bulk-memory GPU work a no-op on the recording side, so a
 * software adapter (lavapipe) has nothing to rasterize.
 *
 * The gate (`bench/scripts/ir_gate.py`) counts guest instructions of two
 * CPU-side roots — `lower` and `encode` — and never consumes rendered
 * output. Those roots make no Vulkan calls at all, so an interception
 * here cannot move their counted Ir. Everything else passes through:
 * command recording, submission, fences, queries and host-visible
 * memory behave exactly as without the layer; only the draw, dispatch,
 * clear, copy, blit and resolve commands in each command buffer are
 * dropped before they can execute on the device.
 */
#include <vulkan/vulkan.h>
#include <vulkan/vk_layer.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define LAYER_NAME "VK_LAYER_CHERENKOV_no_raster"

/* Every recorded command whose execution is real GPU work. Recording
 * still happens — the dispatch chain just drops the call — so command
 * buffers remain complete and valid. Queries that produce data the CPU
 * reads (timestamps, query-pool resolves) are deliberately NOT here. */
static const char *const NOOP_PREFIXES[] = {
    "vkCmdDraw",      /* all draw variants: indexed/indirect/count/multi/mesh */
    "vkCmdDispatch",
    "vkCmdClear",
    "vkCmdCopyAccelerationStructure", /* vkCmdCopyQueryPoolResults must run */
    "vkCmdCopyBuffer", "vkCmdCopyImage", "vkCmdCopyMemory",
    "vkCmdBlit", "vkCmdResolveImage",
    "vkCmdFillBuffer", "vkCmdUpdateBuffer",
    "vkCmdExecuteCommands",
    "vkCmdGenerateMipmap",
    "vkCmdTraceRays", "vkCmdBuild", "vkCmdDecompressMemory",
    "vkCmdCu", /* CUDA kernels */
};

static int is_noop(const char *name) {
    /* Timestamp resolves read query results back to the CPU. */
    if (strcmp(name, "vkCmdCopyQueryPoolResults") == 0)
        return 0;
    for (unsigned i = 0; i < sizeof(NOOP_PREFIXES) / sizeof(NOOP_PREFIXES[0]); i++)
        if (strncmp(name, NOOP_PREFIXES[i], strlen(NOOP_PREFIXES[i])) == 0)
            return 1;
    return 0;
}

static void VKAPI_PTR noop_cmd(void) {}

#define MAX_LINKS 8
static struct {
    VkInstance inst;
    PFN_vkGetInstanceProcAddr gipa;
} inst_links[MAX_LINKS];
static struct {
    VkDevice dev;
    PFN_vkGetDeviceProcAddr gdpa;
} dev_links[MAX_LINKS];

static PFN_vkGetInstanceProcAddr inst_gipa(VkInstance inst) {
    for (int i = 0; i < MAX_LINKS; i++)
        if (inst_links[i].inst == inst)
            return inst_links[i].gipa;
    return NULL;
}

static PFN_vkGetDeviceProcAddr dev_gdpa(VkDevice dev) {
    for (int i = 0; i < MAX_LINKS; i++)
        if (dev_links[i].dev == dev)
            return dev_links[i].gdpa;
    return NULL;
}

VKAPI_ATTR VkResult VKAPI_CALL layer_create_instance(const VkInstanceCreateInfo *ci,
                                                     const VkAllocationCallbacks *alloc,
                                                     VkInstance *out) {
    VkLayerInstanceCreateInfo *chain = (VkLayerInstanceCreateInfo *)ci->pNext;
    while (chain && !(chain->sType == VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO &&
                      chain->function == VK_LAYER_LINK_INFO))
        chain = (VkLayerInstanceCreateInfo *)chain->pNext;
    if (!chain || !chain->u.pLayerInfo)
        return VK_ERROR_INITIALIZATION_FAILED;
    PFN_vkGetInstanceProcAddr next_gipa = chain->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    PFN_vkCreateInstance next_create =
        (PFN_vkCreateInstance)next_gipa(NULL, "vkCreateInstance");
    if (!next_create)
        return VK_ERROR_INITIALIZATION_FAILED;
    chain->u.pLayerInfo = chain->u.pLayerInfo->pNext;
    VkResult res = next_create(ci, alloc, out);
    if (res == VK_SUCCESS) {
        int i = 0;
        while (i < MAX_LINKS && inst_links[i].inst)
            i++;
        if (i == MAX_LINKS) {
            fprintf(stderr, "%s: instance table full (%d live instances)\n",
                    LAYER_NAME, MAX_LINKS);
            abort();
        }
        inst_links[i].inst = *out;
        inst_links[i].gipa = next_gipa;
    }
    return res;
}

VKAPI_ATTR VkResult VKAPI_CALL layer_create_device(VkPhysicalDevice gpu,
                                                   const VkDeviceCreateInfo *ci,
                                                   const VkAllocationCallbacks *alloc,
                                                   VkDevice *out) {
    VkLayerDeviceCreateInfo *chain = (VkLayerDeviceCreateInfo *)ci->pNext;
    while (chain && !(chain->sType == VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO &&
                      chain->function == VK_LAYER_LINK_INFO))
        chain = (VkLayerDeviceCreateInfo *)chain->pNext;
    if (!chain || !chain->u.pLayerInfo)
        return VK_ERROR_INITIALIZATION_FAILED;
    PFN_vkGetInstanceProcAddr next_gipa = chain->u.pLayerInfo->pfnNextGetInstanceProcAddr;
    PFN_vkGetDeviceProcAddr next_gdpa = chain->u.pLayerInfo->pfnNextGetDeviceProcAddr;
    PFN_vkCreateDevice next_create =
        (PFN_vkCreateDevice)next_gipa(NULL, "vkCreateDevice");
    if (!next_create)
        return VK_ERROR_INITIALIZATION_FAILED;
    chain->u.pLayerInfo = chain->u.pLayerInfo->pNext;
    VkResult res = next_create(gpu, ci, alloc, out);
    if (res == VK_SUCCESS) {
        int i = 0;
        while (i < MAX_LINKS && dev_links[i].dev)
            i++;
        if (i == MAX_LINKS) {
            fprintf(stderr, "%s: device table full (%d live devices)\n",
                    LAYER_NAME, MAX_LINKS);
            abort();
        }
        dev_links[i].dev = *out;
        dev_links[i].gdpa = next_gdpa;
    }
    return res;
}

VKAPI_ATTR void VKAPI_CALL layer_destroy_instance(VkInstance inst,
                                                  const VkAllocationCallbacks *alloc) {
    PFN_vkGetInstanceProcAddr gipa = inst_gipa(inst);
    if (!gipa) {
        fprintf(stderr, "%s: vkDestroyInstance on unregistered instance %p\n",
                LAYER_NAME, (void *)inst);
        abort();
    }
    PFN_vkDestroyInstance next =
        (PFN_vkDestroyInstance)gipa(inst, "vkDestroyInstance");
    for (int i = 0; i < MAX_LINKS; i++)
        if (inst_links[i].inst == inst) {
            inst_links[i].inst = NULL;
            inst_links[i].gipa = NULL;
        }
    if (next)
        next(inst, alloc);
}

VKAPI_ATTR void VKAPI_CALL layer_destroy_device(VkDevice dev,
                                                const VkAllocationCallbacks *alloc) {
    PFN_vkGetDeviceProcAddr gdpa = dev_gdpa(dev);
    if (!gdpa) {
        fprintf(stderr, "%s: vkDestroyDevice on unregistered device %p\n",
                LAYER_NAME, (void *)dev);
        abort();
    }
    PFN_vkDestroyDevice next = (PFN_vkDestroyDevice)gdpa(dev, "vkDestroyDevice");
    for (int i = 0; i < MAX_LINKS; i++)
        if (dev_links[i].dev == dev) {
            dev_links[i].dev = NULL;
            dev_links[i].gdpa = NULL;
        }
    if (next)
        next(dev, alloc);
}

VKAPI_ATTR VkResult VKAPI_CALL layer_enumerate_instance_layer_properties(
    uint32_t *count, VkLayerProperties *props) {
    if (props) {
        memset(props, 0, sizeof(*props));
        strncpy(props->layerName, LAYER_NAME, sizeof(props->layerName) - 1);
        props->specVersion = VK_MAKE_VERSION(1, 3, 0);
        props->implementationVersion = 1;
        strncpy(props->description, "Ir gate: no GPU-side command execution",
                sizeof(props->description) - 1);
    }
    *count = 1;
    return VK_SUCCESS;
}

VKAPI_ATTR VkResult VKAPI_CALL layer_enumerate_instance_extension_properties(
    const char *layer_name, uint32_t *count, VkExtensionProperties *props) {
    (void)props;
    if (layer_name && strcmp(layer_name, LAYER_NAME) == 0) {
        *count = 0;
        return VK_SUCCESS;
    }
    return VK_ERROR_LAYER_NOT_PRESENT;
}

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL layer_get_device_proc_addr(VkDevice dev,
                                                                    const char *name) {
    if (is_noop(name))
        return (PFN_vkVoidFunction)noop_cmd;
    if (strcmp(name, "vkGetDeviceProcAddr") == 0)
        return (PFN_vkVoidFunction)layer_get_device_proc_addr;
    if (strcmp(name, "vkDestroyDevice") == 0)
        return (PFN_vkVoidFunction)layer_destroy_device;
    PFN_vkGetDeviceProcAddr gdpa = dev_gdpa(dev);
    return gdpa ? gdpa(dev, name) : NULL;
}

/* The loader resolves every entrypoint through vkGetInstanceProcAddr,
 * including device-level ones, so the no-op names must be answered here
 * as well. */
VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL layer_get_instance_proc_addr(VkInstance inst,
                                                                      const char *name) {
    if (strcmp(name, "vkGetInstanceProcAddr") == 0)
        return (PFN_vkVoidFunction)layer_get_instance_proc_addr;
    if (strcmp(name, "vkGetDeviceProcAddr") == 0)
        return (PFN_vkVoidFunction)layer_get_device_proc_addr;
    if (strcmp(name, "vkCreateInstance") == 0)
        return (PFN_vkVoidFunction)layer_create_instance;
    if (strcmp(name, "vkCreateDevice") == 0)
        return (PFN_vkVoidFunction)layer_create_device;
    if (strcmp(name, "vkDestroyInstance") == 0)
        return (PFN_vkVoidFunction)layer_destroy_instance;
    if (strcmp(name, "vkEnumerateInstanceLayerProperties") == 0)
        return (PFN_vkVoidFunction)layer_enumerate_instance_layer_properties;
    if (strcmp(name, "vkEnumerateInstanceExtensionProperties") == 0)
        return (PFN_vkVoidFunction)layer_enumerate_instance_extension_properties;
    if (is_noop(name))
        return (PFN_vkVoidFunction)noop_cmd;
    PFN_vkGetInstanceProcAddr gipa = inst_gipa(inst);
    return gipa ? gipa(inst, name) : NULL;
}

/* Exported entrypoints the loader binds by name. */
VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetInstanceProcAddr(VkInstance inst,
                                                               const char *name) {
    return layer_get_instance_proc_addr(inst, name);
}

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetDeviceProcAddr(VkDevice dev,
                                                           const char *name) {
    return layer_get_device_proc_addr(dev, name);
}
