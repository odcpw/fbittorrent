// Native input for the owned headless compositor only. No default-display fallback.
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "virtual-pointer.h"

static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version) {
    (void)data;
    if (!strcmp(interface, "zwlr_virtual_pointer_manager_v1"))
        manager = wl_registry_bind(registry, name, &zwlr_virtual_pointer_manager_v1_interface,
                                   version < 2 ? version : 2);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}
static uint32_t millis(void) {
    struct timespec value;
    clock_gettime(CLOCK_MONOTONIC, &value);
    return (uint32_t)(value.tv_sec * 1000 + value.tv_nsec / 1000000);
}
static void frame(struct wl_display *display, struct zwlr_virtual_pointer_v1 *pointer) {
    zwlr_virtual_pointer_v1_frame(pointer);
    if (wl_display_roundtrip(display) < 0) { perror("private pointer frame"); exit(1); }
}
int main(int argc, char **argv) {
    alarm(12);
    const char *runtime = getenv("XDG_RUNTIME_DIR"), *private_net = getenv("FBT_PRIVATE_NET_ACTIVE");
    if (argc != 8 || !runtime || !private_net || strcmp(private_net, "1") ||
        strncmp(runtime, "/tmp/fbt-d-", 11) || strncmp(argv[1], runtime, strlen(runtime)) ||
        argv[1][strlen(runtime)] != '/' || strstr(argv[1], "..")) {
        fprintf(stderr, "private-pointer: explicit isolated display and private network required\n"); return 2;
    }
    uint32_t values[6];
    for (int i = 0; i < 6; ++i) {
        char *end; errno = 0; unsigned long value = strtoul(argv[i+2], &end, 10);
        if (errno || *end || value > 10000) return 2;
        values[i] = (uint32_t)value;
    }
    uint32_t x=values[0], y=values[1], width=values[2], height=values[3], button=values[4], clicks=values[5];
    if (!width || !height || x>=width || y>=height || clicks>2 || (button!=272 && button!=273)) return 2;
    struct wl_display *display=wl_display_connect(argv[1]);
    if (!display) { perror("private pointer connect"); return 1; }
    struct wl_registry *registry=wl_display_get_registry(display);
    const struct wl_registry_listener listener={global,removed};
    wl_registry_add_listener(registry,&listener,NULL);
    if (wl_display_roundtrip(display)<0 || !manager) { fprintf(stderr,"private virtual pointer unavailable\n"); return 1; }
    struct zwlr_virtual_pointer_v1 *pointer=zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager,NULL);
    zwlr_virtual_pointer_v1_motion_absolute(pointer,millis(),x,y,width,height);frame(display,pointer);
    puts("ready");fflush(stdout);
    int command;
    while ((command=getchar())!=EOF && command!='q') {
        if(command=='m') {zwlr_virtual_pointer_v1_motion_absolute(pointer,millis(),x,y,width,height);frame(display,pointer);}
        else if(command=='b') {
            for (uint32_t i=0;i<clicks;++i) {
                zwlr_virtual_pointer_v1_button(pointer,millis(),button,WL_POINTER_BUTTON_STATE_PRESSED);frame(display,pointer);usleep(25000);
                zwlr_virtual_pointer_v1_button(pointer,millis(),button,WL_POINTER_BUTTON_STATE_RELEASED);frame(display,pointer);usleep(25000);
            }
            puts("clicked");fflush(stdout);
        }
    }
    zwlr_virtual_pointer_v1_destroy(pointer);zwlr_virtual_pointer_manager_v1_destroy(manager);
    wl_registry_destroy(registry);wl_display_flush(display);wl_display_disconnect(display);
    return 0;
}
