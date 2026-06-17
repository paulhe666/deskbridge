#ifndef DESKBRIDGE_MACOS_NATIVE_H
#define DESKBRIDGE_MACOS_NATIVE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct DeskbridgeHidContext DeskbridgeHidContext;

enum {
    DESKBRIDGE_MOUSE_MOVED = 1,
    DESKBRIDGE_MOUSE_LEFT_DOWN = 2,
    DESKBRIDGE_MOUSE_LEFT_UP = 3,
    DESKBRIDGE_MOUSE_RIGHT_DOWN = 4,
    DESKBRIDGE_MOUSE_RIGHT_UP = 5,
    DESKBRIDGE_MOUSE_OTHER_DOWN = 6,
    DESKBRIDGE_MOUSE_OTHER_UP = 7,
    DESKBRIDGE_MOUSE_LEFT_DRAGGED = 8,
    DESKBRIDGE_MOUSE_RIGHT_DRAGGED = 9,
    DESKBRIDGE_MOUSE_OTHER_DRAGGED = 10,
};

enum {
    DESKBRIDGE_MOUSE_BUTTON_LEFT = 0,
    DESKBRIDGE_MOUSE_BUTTON_RIGHT = 1,
    DESKBRIDGE_MOUSE_BUTTON_CENTER = 2,
};

int32_t deskbridge_hid_context_create(
    DeskbridgeHidContext **context,
    size_t *keyboard_count,
    size_t *mouse_count);

void deskbridge_hid_context_destroy(DeskbridgeHidContext *context);

int32_t deskbridge_hid_post_key(
    DeskbridgeHidContext *context,
    uint16_t keycode,
    bool down,
    uint64_t flags,
    bool autorepeat);

int32_t deskbridge_hid_post_mouse(
    DeskbridgeHidContext *context,
    uint8_t kind,
    uint8_t button,
    double x,
    double y,
    int64_t click_count,
    int32_t dx,
    int32_t dy);

int32_t deskbridge_hid_post_scroll(
    DeskbridgeHidContext *context,
    int32_t horizontal,
    int32_t vertical);

void deskbridge_main_display_size(uint32_t *width, uint32_t *height);

#ifdef __cplusplus
}
#endif

#endif
