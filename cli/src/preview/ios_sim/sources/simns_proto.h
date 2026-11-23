#pragma once

#include <mach/mach.h>
#include <stdint.h>

#define WATERUI_SIMNS_NAME_MAX 128

enum {
    WATERUI_SIMNS_MSG_HELLO = 0x57490001u,
    WATERUI_SIMNS_MSG_REGISTER = 0x57490002u,
    WATERUI_SIMNS_MSG_READY = 0x57490003u,
    WATERUI_SIMNS_MSG_NEW_NS = 0x57490004u,
    WATERUI_SIMNS_MSG_NS_REPLY = 0x57490005u,
};

enum {
    WATERUI_SIMNS_F_HOST_SIDE = 1u << 0,
};

typedef struct {
    mach_msg_header_t h;
    mach_msg_body_t b;
    mach_msg_port_descriptor_t port;
} waterui_simns_port_msg_t;

typedef struct {
    mach_msg_header_t h;
    mach_msg_body_t b;
    mach_msg_port_descriptor_t port;
    uint32_t flags;
    char name[WATERUI_SIMNS_NAME_MAX];
} waterui_simns_register_msg_t;
