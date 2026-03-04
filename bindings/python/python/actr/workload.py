class WorkloadBase:
    """
    Minimal Workload base class.

    Provides get_dispatcher() so generated workloads can be simple wrappers.
    """

    def __init__(self, dispatcher):
        self._dispatcher = dispatcher

    def get_dispatcher(self):
        return self._dispatcher
