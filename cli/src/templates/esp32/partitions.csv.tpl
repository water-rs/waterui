# Name,   Type, SubType, Offset,  Size
nvs,      data, nvs,     0x9000,  0x6000,
phy_init, data, phy,     0xf000,  0x1000,
factory,  app,  factory, {{ ctx.esp32.app_partition_offset }}, {{ ctx.esp32.app_partition_size }},
