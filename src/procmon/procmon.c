#include <sys/socket.h>
#include <linux/netlink.h>
#include <linux/connector.h>
#include <linux/cn_proc.h>
#include <signal.h>
#include <errno.h>
#include <stdbool.h>
#include <unistd.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

int nl_connect()
{
    int rc;
    int nl_socket;
    struct sockaddr_nl sa_nl;

    nl_socket = socket(PF_NETLINK, SOCK_DGRAM, NETLINK_CONNECTOR);
    if (nl_socket == -1)
    {
        return -1;
    }

    sa_nl.nl_family = AF_NETLINK;
    sa_nl.nl_groups = CN_IDX_PROC;
    sa_nl.nl_pid = getpid();

    rc = bind(nl_socket, (struct sockaddr *)&sa_nl, sizeof(sa_nl));
    if (rc == -1)
    {
        close(nl_socket);
        return -1;
    }

    return nl_socket;
}

int update_nl(int nl_socket, bool enable)
{
    int rc;
    struct __attribute__((aligned(NLMSG_ALIGNTO)))
    {
        struct nlmsghdr nl_hdr;
        struct __attribute__((__packed__))
        {
            struct cn_msg cn_msg;
            enum proc_cn_mcast_op cn_mcast;
        };
    } nlcn_msg;

    memset(&nlcn_msg, 0, sizeof(nlcn_msg));

    nlcn_msg.nl_hdr.nlmsg_len = sizeof(nlcn_msg);
    nlcn_msg.nl_hdr.nlmsg_pid = getpid();
    nlcn_msg.nl_hdr.nlmsg_type = NLMSG_DONE;

    nlcn_msg.cn_msg.id.idx = CN_IDX_PROC;
    nlcn_msg.cn_msg.id.val = CN_VAL_PROC;
    nlcn_msg.cn_msg.len = sizeof(enum proc_cn_mcast_op);

    nlcn_msg.cn_mcast = enable ? PROC_CN_MCAST_LISTEN : PROC_CN_MCAST_IGNORE;

    rc = send(nl_socket, &nlcn_msg, sizeof(nlcn_msg), 0);
    if (rc == -1)
    {
        return -1;
    }

    return 0;
}

struct Event
{
    unsigned event_type;
    pid_t pid,
        ppid,
        tgid;
};

int wait_for_event(int nl_sock, struct Event *event)
{
    int rc;
    struct __attribute__((aligned(NLMSG_ALIGNTO)))
    {
        struct nlmsghdr nl_hdr;
        struct __attribute__((__packed__))
        {
            struct cn_msg cn_msg;
            struct proc_event proc_ev;
        };
    } nlcn_msg;

    rc = recv(nl_sock, &nlcn_msg, sizeof(nlcn_msg), 0);
    if (rc == 0)
    {
        return 0;
    }
    else if (rc == -1)
    {
        return -1;
    }

    event->event_type = nlcn_msg.proc_ev.what;

    switch (nlcn_msg.proc_ev.what)
    {
    case PROC_EVENT_FORK:
        event->pid = nlcn_msg.proc_ev.event_data.fork.child_pid;
        event->ppid = nlcn_msg.proc_ev.event_data.fork.parent_pid;
        event->tgid = nlcn_msg.proc_ev.event_data.fork.child_tgid;
        break;

    case PROC_EVENT_EXEC:
        event->pid = nlcn_msg.proc_ev.event_data.exec.process_pid;
        event->tgid = nlcn_msg.proc_ev.event_data.exec.process_tgid;
        break;

    case PROC_EVENT_EXIT:
        event->pid = nlcn_msg.proc_ev.event_data.exit.process_pid;
        event->tgid = nlcn_msg.proc_ev.event_data.exit.process_pid;
        break;

    default:
        break;
    }

    return 0;
}