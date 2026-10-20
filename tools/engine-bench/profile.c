/* Linux/glibc allocation counter, ONLY for separate diagnostic runs.
 * A shared counter file survives abnormal termination; no signal handlers.
 * Build with the documented release wrapper: cc -shared -fPIC -O2 ... -ldl
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t, size_t);
extern void *__libc_realloc(void *, size_t);
extern void *__libc_memalign(size_t, size_t);
static _Atomic int enabled;
static _Atomic uint64_t *counters;
static void count(size_t size) {
    if (atomic_load_explicit(&enabled, memory_order_relaxed)) {
        atomic_fetch_add_explicit(&counters[0], 1, memory_order_relaxed);
        atomic_fetch_add_explicit(&counters[1], size, memory_order_relaxed);
    }
}
void *malloc(size_t size) { count(size); return __libc_malloc(size); }
void *calloc(size_t n, size_t size) { count(n*size); return __libc_calloc(n,size); }
void *realloc(void *p, size_t size) { count(size); return __libc_realloc(p,size); }
void *memalign(size_t alignment, size_t size) { count(size); return __libc_memalign(alignment,size); }
void *aligned_alloc(size_t alignment, size_t size) { return memalign(alignment,size); }
int posix_memalign(void **out, size_t alignment, size_t size) {
    if (alignment < sizeof(void*) || (alignment & (alignment-1))) return 22;
    void *p=memalign(alignment,size); if (!p) return 12; *out=p; return 0;
}
__attribute__((constructor)) static void init(void) {
    const char *path=getenv("FBT_BENCH_COUNTERS"); if(!path) return;
    // The daemon's helper execs inherit its environment. They must neither
    // truncate the parent's live counters nor count as engine allocations.
    if(getenv("FBT_BENCH_COUNTER_OWNER")) return;
    char owner[32]; snprintf(owner,sizeof(owner),"%ld",(long)getpid());
    setenv("FBT_BENCH_COUNTER_OWNER",owner,1);
    char live[4096]; snprintf(live,sizeof(live),"%s.live",path);
    int fd=open(live,O_RDWR|O_CREAT|O_TRUNC|O_CLOEXEC,0600);
    if(fd<0 || ftruncate(fd,16)) _exit(111);
    counters=mmap(NULL,16,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0); close(fd);
    if(counters==MAP_FAILED) _exit(112);
    atomic_store(&enabled,1);
}
__attribute__((destructor)) static void finish(void) {
    if(!atomic_exchange(&enabled,0)) return;
    uint64_t allocations=atomic_load(&counters[0]), bytes=atomic_load(&counters[1]);
    FILE *out=fopen(getenv("FBT_BENCH_COUNTERS"),"w"); if(!out) return;
    fprintf(out,"{\"allocations\":%lu,\"requested_bytes\":%lu}\n",allocations,bytes);
    fclose(out); munmap((void*)counters,16);
}
