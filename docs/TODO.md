# What remains to be done

NIX: add a sytemd unit for user so that it auto-starts in web mode

UX: provide model info to frontend

CFG: add auth headers + cookies

QC: implement pathological cases here if needed (huge payload) dans opeanai::send_chat_request

UX: ask confirmation before leaving (ctrl-c) in cli::run_chat

UX: apply theming to prompt::select_model

QC: allow to use different OpenAI services level in openai::send_chat_request : for now, Flex => "Invalid service_tier argument" et Priority => répone Some(Default)
