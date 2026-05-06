# What remains to be done

UX: provide model info to frontend

CFG: add auth headers + cookies

QC: implement pathological cases here if needed (huge payload) dans opeanai::send_chat_request

UX: ask confirmation before leaving (ctrl-c) in cli::run_chat

UX: apply theming to prompt::select_model

QC: allow to use different OpenAI services level in openai::send_chat_request : for now, Flex => "Invalid service_tier argument" et Priority => répone Some(Default)

WIN: make "as a service" work :

    install:

    2026-05-06T14:02:10.609098Z  WARN service_manager::sc: sc.exe does not support automatic restart policies through 'sc create'; service 'com.github.nipil.openai-compatible-cli-chat' will not restart automatically. Use 'sc failure' to configure restart behavior manually.
    Service com.github.nipil.openai-compatible-cli-chat installed.
    --> install works

    2026-05-06T14:02:10.631307Z DEBUG native::service: Service label label=ServiceLabel { qualifier: Some("com"), organization: Some("github"), application: "nipil.openai-compatible-cli-chat" }
    Error: 2026-05-06T14:02:10.657287Z DEBUG Connection{peer=Client}: h2::codec::framed_write: send frame=GoAway { error_code: NO_ERROR, last_stream_id: StreamId(0) }
    Service manager error: Command failed with exit code 1053: [SC] StartService ‚chec(s) 1053 :
    Le service n'a pas r‚pondu assez vite … la demande de lancement ou de contr“le.
    --> start does not ?
    ====> possible cause : windows requires a logout so that the services can work for the user ?

    uninstall:

    Command failed with exit code 1062: [SC] ControlService ‚chec(s) 1062 :
    Le service n'a pas ‚t‚ d‚marr‚.
    --> uninstall is not done because stop fails (as it is not started)
