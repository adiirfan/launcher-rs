// Feeds the XZ3 touchscreen a finger at a steady 125 reports a second.
// touchgen X0 Y0 [X Y MS]...  : press at X0,Y0, then glide to each X,Y over
// MS milliseconds (same point = hold still), then lift.
#include <fcntl.h>
#include <linux/input.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>
static int fd;
static void ev(int t, int c, int v) { struct input_event e = {0}; e.type = t; e.code = c; e.value = v; write(fd, &e, sizeof e); }
static void at(int x, int y) { ev(3, 57, 7); ev(3, 53, x); ev(3, 54, y); ev(3, 58, 60); ev(0, 2, 0); ev(0, 0, 0); }
static struct timespec next;
static void tick(void) { next.tv_nsec += 8000000; if (next.tv_nsec >= 1000000000) { next.tv_nsec -= 1000000000; next.tv_sec++; } clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, 0); }
int main(int argc, char **argv) {
    fd = open("/dev/input/event6", O_WRONLY);
    if (fd < 0 || argc < 3) return 1;
    int x = atoi(argv[1]), y = atoi(argv[2]);
    clock_gettime(CLOCK_MONOTONIC, &next);
    at(x, y);
    for (int i = 3; i + 2 < argc; i += 3) {
        int tx = atoi(argv[i]), ty = atoi(argv[i + 1]), n = atoi(argv[i + 2]) / 8;
        for (int k = 1; k <= n; k++) { tick(); at(x + (tx - x) * k / n, y + (ty - y) * k / n); }
        x = tx; y = ty;
    }
    tick(); ev(0, 2, 0); ev(0, 0, 0);
    return 0;
}
