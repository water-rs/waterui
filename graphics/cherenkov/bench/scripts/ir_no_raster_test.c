#include <vulkan/vulkan.h>
#include <assert.h>
#include <dlfcn.h>
#include <stdlib.h>
#include <string.h>

/* Like ash, load Vulkan locally rather than linking it into global scope. */
int main(int argc, char **argv) {
    void *library = dlopen("libvulkan.so.1", RTLD_NOW | RTLD_LOCAL);
    assert(library);
    PFN_vkGetInstanceProcAddr gipa = dlsym(library, "vkGetInstanceProcAddr");
    assert(gipa);
    PFN_vkCreateInstance create = (PFN_vkCreateInstance)gipa(NULL, "vkCreateInstance");
    VkApplicationInfo app = {.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                             .apiVersion = VK_API_VERSION_1_3};
    VkInstanceCreateInfo ci = {.sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
                              .pApplicationInfo = &app};
    /* Exceed the layer's live-handle table size to catch leaked registrations. */
    for (int i = 0; i < 12; i++) {
        VkInstance instance;
        assert(create(&ci, NULL, &instance) == VK_SUCCESS);
        PFN_vkDestroyInstance destroy = (PFN_vkDestroyInstance)gipa(instance, "vkDestroyInstance");
        if (argc == 2 && strcmp(argv[1], "unregistered") == 0) {
            /* The layer rejects this before dereferencing the fake handle. */
            void *layer = dlopen(getenv("CHERENKOV_TEST_LAYER"), RTLD_NOW | RTLD_NOLOAD);
            assert(layer);
            PFN_vkGetInstanceProcAddr layer_gipa = dlsym(layer, "cherenkovGetInstanceProcAddr");
            assert(layer_gipa);
            PFN_vkDestroyInstance layer_destroy = (PFN_vkDestroyInstance)layer_gipa(instance, "vkDestroyInstance");
            layer_destroy((VkInstance)(uintptr_t)1, NULL);
            return 1;
        }
        PFN_vkEnumeratePhysicalDevices enumerate =
            (PFN_vkEnumeratePhysicalDevices)gipa(instance, "vkEnumeratePhysicalDevices");
        uint32_t count = 1;
        VkPhysicalDevice gpu;
        VkResult result = enumerate(instance, &count, &gpu);
        assert((result == VK_SUCCESS || result == VK_INCOMPLETE) && count);
        float priority = 1;
        VkDeviceQueueCreateInfo queue = {.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                        .queueFamilyIndex = 0, .queueCount = 1,
                                        .pQueuePriorities = &priority};
        VkDeviceCreateInfo device_ci = {.sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO,
                                       .queueCreateInfoCount = 1, .pQueueCreateInfos = &queue};
        PFN_vkCreateDevice create_device = (PFN_vkCreateDevice)gipa(instance, "vkCreateDevice");
        VkDevice device;
        assert(create_device(gpu, &device_ci, NULL, &device) == VK_SUCCESS);
        PFN_vkGetDeviceProcAddr gdpa = (PFN_vkGetDeviceProcAddr)gipa(instance, "vkGetDeviceProcAddr");
        PFN_vkVoidFunction draw = gdpa(device, "vkCmdDraw");
        assert(draw && draw == gdpa(device, "vkCmdCopyBuffer"));
        PFN_vkDestroyDevice destroy_device = (PFN_vkDestroyDevice)gdpa(device, "vkDestroyDevice");
        destroy_device(device, NULL);
        destroy(instance, NULL);
    }
    dlclose(library);
    return 0;
}
