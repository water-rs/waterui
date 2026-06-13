# Rendering happens on the main task; give it room for the CPU
# rasterization stack.
CONFIG_ESP_MAIN_TASK_STACK_SIZE=163840

# Frames can exceed the default 5 s task watchdog while logging.
CONFIG_ESP_TASK_WDT_INIT=n

CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y

# QEMU lacks the ESP32-S3 permission-control hardware; memory protection
# must be off for emulated runs.
CONFIG_ESP_SYSTEM_MEMPROT_FEATURE=n

# Boards with native USB expose the USB-Serial-JTAG console; route logs
# there so they arrive over the USB cable.
CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y
