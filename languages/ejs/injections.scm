; Inject JavaScript into the code inside <% %>, <%= %>, <%- %>
((directive
   (code) @injection.content)
 (#set! injection.language "javascript"))

((output_directive
   (code) @injection.content)
 (#set! injection.language "javascript"))

; Inject HTML into the literal template content outside of <% %> tags
((content) @injection.content
 (#set! injection.language "html"))
