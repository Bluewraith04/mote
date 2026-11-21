#ifndef PANE_WIN32_H
#define PANE_WIN32_H

#include <stdint.h>

typedef struct PaneMutex { void *ptr; } PaneMutex;
typedef struct PaneCond { void *ptr; } PaneCond;
typedef void *PaneThread;

void pw_mutex_init(PaneMutex *m);
void pw_mutex_lock(PaneMutex *m);
void pw_mutex_unlock(PaneMutex *m);
void pw_mutex_destroy(PaneMutex *m);

void pw_cond_init(PaneCond *c);
void pw_cond_wait(PaneCond *c, PaneMutex *m);
void pw_cond_broadcast(PaneCond *c);

int pw_thread_create(PaneThread *t, void *(*fn)(void *), void *arg);
void pw_thread_join(PaneThread t);

#endif
