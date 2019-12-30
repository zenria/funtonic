
// Implementation code where T is the returned data shape
function api<T>(url: string): Promise<T> {
    return fetch(url)
        .then(response => {
            if (!response.ok) {
                throw new Error(response.statusText)
            }
            return response.json() as Promise<T>
        })

}

export type Version = {
    director: String,
    core: String,
    protocol: String,
    query_parser: String,
}

export function version(): Promise<Version> {
    return api("/api/version");
}
