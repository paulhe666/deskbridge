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

enum {
    DESKBRIDGE_TAP_SCROLL = 20,
    DESKBRIDGE_TAP_KEY_DOWN = 21,
    DESKBRIDGE_TAP_KEY_UP = 22,
    DESKBRIDGE_TAP_FLAGS_CHANGED = 23,
};

typedef bool (*DeskbridgeEventTapCallback)(
    void *context,
    uint32_t kind,
    int64_t a,
    int64_t b,
    int64_t c,
    int64_t d,
    double x,
    double y);

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

int32_t deskbridge_hid_cycle_keyboard_input_source(DeskbridgeHidContext *context);

int32_t deskbridge_event_tap_run(
    void *context,
    DeskbridgeEventTapCallback callback);

int32_t deskbridge_macos_set_cursor_position(double x, double y);

int32_t deskbridge_macos_hide_cursor(void);

int32_t deskbridge_macos_show_cursor(void);

void deskbridge_main_display_size(uint32_t *width, uint32_t *height);

#ifdef __cplusplus
}
#endif

#endif
