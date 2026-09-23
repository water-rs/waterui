# Rendering happens on the main task; give it room for the CPU
# rasterization stack. Sized for the target chip's SRAM.
CONFIG_ESP_MAIN_TASK_STACK_SIZE={{ ctx.esp32.main_task_stack_bytes }}

# Frames can exceed the default 5 s task watchdog while logging.
CONFIG_ESP_TASK_WDT_INIT=n

CONFIG_ESPTOOLPY_FLASHSIZE_{{ ctx.esp32.flash_size_mb }}MB=y

# QEMU lacks the permission-control hardware; memory protection must be off
# for emulated runs.
CONFIG_ESP_SYSTEM_MEMPROT_FEATURE=n
{%- if ctx.esp32.console_uart_default %}

# QEMU surfaces UART0; route the console there so emulated logs reach stdout.
# (Real boards with native USB use the USB-Serial-JTAG console instead.)
CONFIG_ESP_CONSOLE_UART_DEFAULT=y
{%- else %}

# Boards with native USB expose the USB-Serial-JTAG console; route logs
# there so they arrive over the USB cable.
CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y
{%- endif %}
