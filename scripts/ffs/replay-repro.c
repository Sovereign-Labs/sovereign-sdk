#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/futex.h>
#include <linux/io_uring.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifdef REPRO_FUTEX
static _Atomic int word;
static _Atomic int ready;

static void handler(int sig) { (void)sig; }

static void *waiter(void *arg) {
    (void)arg;
    atomic_fetch_add(&ready, 1);
    while (!atomic_load(&word)) {
        int rc = syscall(SYS_futex, &word, FUTEX_WAIT_PRIVATE, 0, NULL, NULL, 0);
        assert(rc == 0 || errno == EAGAIN || errno == EINTR);
    }
    return NULL;
}

int main(void) {
    // FFS suppresses this signal as an internal preemption. Native runs handle it normally.
    struct sigaction sa = {.sa_handler = handler, .sa_flags = SA_RESTART};
    sigemptyset(&sa.sa_mask);
    assert(sigaction(SIGSTKFLT, &sa, NULL) == 0);
    pthread_t threads[16];
    for (int i = 0; i < 16; i++) assert(pthread_create(&threads[i], NULL, waiter, NULL) == 0);
    while (atomic_load(&ready) != 16) sched_yield();
    for (int i = 0; i < 16; i++) assert(pthread_kill(threads[i], SIGSTKFLT) == 0);
    atomic_store(&word, 1);
    assert(syscall(SYS_futex, &word, FUTEX_WAKE_PRIVATE, 16, NULL, NULL, 0) >= 0);
    for (int i = 0; i < 16; i++) assert(pthread_join(threads[i], NULL) == 0);
}
#else
int main(int argc, char **argv) {
    (void)argc;
    struct io_uring_params params = {0};
    int ring = syscall(SYS_io_uring_setup, 4, &params);
    assert(ring >= 0);
    assert(params.features & IORING_FEAT_SINGLE_MMAP);
    size_t sq_size = params.sq_off.array + params.sq_entries * sizeof(unsigned);
    size_t cq_size = params.cq_off.cqes + params.cq_entries * sizeof(struct io_uring_cqe);
    size_t size = sq_size > cq_size ? sq_size : cq_size;
    char *queue = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, ring, IORING_OFF_SQ_RING);
    assert(queue != MAP_FAILED);
    size_t entries_size = params.sq_entries * sizeof(struct io_uring_sqe);
    struct io_uring_sqe *entries = mmap(NULL, entries_size, PROT_READ | PROT_WRITE,
                                      MAP_SHARED, ring, IORING_OFF_SQES);
    assert(entries != MAP_FAILED);
    unsigned *sq_tail = (unsigned *)(queue + params.sq_off.tail);
    unsigned *sq_mask = (unsigned *)(queue + params.sq_off.ring_mask);
    unsigned *sq_array = (unsigned *)(queue + params.sq_off.array);
    unsigned *cq_head = (unsigned *)(queue + params.cq_off.head);
    _Atomic unsigned *cq_tail = (_Atomic unsigned *)(queue + params.cq_off.tail);
    unsigned *cq_mask = (unsigned *)(queue + params.cq_off.ring_mask);
    struct io_uring_cqe *completions = (struct io_uring_cqe *)(queue + params.cq_off.cqes);
    int file = open(argv[0], O_RDONLY);
    assert(file >= 0);
    char buffers[2][4096];
    for (int round = 0; round < 30; round++) {
        for (unsigned request = 0; request < 2; request++) {
            unsigned index = *sq_tail & *sq_mask;
            entries[index] = (struct io_uring_sqe){
                .opcode = IORING_OP_READ, .flags = IOSQE_ASYNC, .fd = file,
                .addr = (unsigned long)buffers[request], .len = sizeof(buffers[request]),
                .user_data = request,
            };
            sq_array[index] = index;
            atomic_fetch_add_explicit((_Atomic unsigned *)sq_tail, 1, memory_order_release);
            assert(syscall(SYS_io_uring_enter, ring, 1, 0, 0, NULL, 0) == 1);
        }
        // An existing completion can satisfy FFS's forced min_complete=1 while the second read is pending.
        unsigned head = *cq_head;
        while (atomic_load_explicit(cq_tail, memory_order_acquire) - head < 2) sched_yield();
        for (unsigned i = 0; i < 2; i++) assert(completions[(head + i) & *cq_mask].res > 0);
        atomic_store_explicit((_Atomic unsigned *)cq_head, head + 2, memory_order_release);
    }
    assert(close(file) == 0);
    assert(munmap(entries, entries_size) == 0);
    assert(munmap(queue, size) == 0);
    assert(close(ring) == 0);
}
#endif
